//! `MavClient`: request/response on top of a [`Link`], and the
//! [`FlightController`] implementation.

use crate::apm::*;
use crate::apm::MavParamType;
use crate::link::{Frame, Link};
use crate::params::{name_of, param_id, values_match, ParamStore};
use crate::{logs, GCS_COMPONENT_ID, GCS_SYSTEM_ID};
use domain::ap_consts::*;
use domain::fc::*;
use domain::{Firmware, Recommendation, Tune};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// ArduPilot's PARAM_REQUEST_LIST streams ≤ 5 params per scheduler tick without
/// flow control (GCS_Param.cpp queued_param_send); 1 000 params ≈ 3–6 s.
const PARAM_LIST_IDLE: Duration = Duration::from_millis(2500);
const PARAM_READ_TIMEOUT: Duration = Duration::from_millis(700);
const ACK_TIMEOUT: Duration = Duration::from_millis(1500);
const RETRIES: u32 = 3;

pub struct MavClient {
    link: Box<dyn Link>,
    address: String,
    target: (u8, u8),
    pub store: ParamStore,
    last_heartbeat: Option<(Instant, HEARTBEAT_DATA)>,
    last_heartbeat_sent: Instant,
    version: Option<AUTOPILOT_VERSION_DATA>,
    banner: Option<String>,
    pub statustext: Vec<String>,
    reopen: Option<Box<dyn Fn(&str) -> Result<Box<dyn Link>, FcError> + Send>>,
}

fn fw_version_string(v: u32) -> String {
    // GCS_Common.cpp send_autopilot_version: major<<24 | minor<<16 | patch<<8 | fw_type
    let (maj, min, pat, ty) = ((v >> 24) & 0xff, (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    let suffix = match ty {
        0 => "-dev",
        64 => "-alpha",
        128 => "-beta",
        192 => "-rc",
        _ => "",
    };
    format!("{maj}.{min}.{pat}{suffix}")
}

fn is_copter(t: MavType) -> bool {
    matches!(
        t,
        MavType::MAV_TYPE_QUADROTOR | MavType::MAV_TYPE_HEXAROTOR | MavType::MAV_TYPE_OCTOROTOR | MavType::MAV_TYPE_TRICOPTER | MavType::MAV_TYPE_COAXIAL | MavType::MAV_TYPE_HELICOPTER | MavType::MAV_TYPE_DODECAROTOR | MavType::MAV_TYPE_DECAROTOR
    )
}

impl MavClient {
    /// Open `address` (see [`crate::link::open`]), wait for the autopilot heartbeat,
    /// then ask for AUTOPILOT_VERSION and the banner.
    pub fn open(address: &str, timeout: Duration) -> Result<Self, FcError> {
        let link = crate::link::open(address)?;
        let mut c = Self::with_link(link, address, timeout)?;
        c.reopen = Some(Box::new(|a| crate::link::open(a)));
        Ok(c)
    }

    pub fn with_link(link: Box<dyn Link>, address: &str, timeout: Duration) -> Result<Self, FcError> {
        let mut c = Self {
            link,
            address: address.to_string(),
            target: (1, 1),
            store: ParamStore::default(),
            last_heartbeat: None,
            last_heartbeat_sent: Instant::now() - Duration::from_secs(2),
            version: None,
            banner: None,
            statustext: Vec::new(),
            reopen: None,
        };
        c.wait_heartbeat(timeout)?;
        c.request_version();
        Ok(c)
    }

    pub fn target(&self) -> (u8, u8) {
        self.target
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn send(&mut self, msg: &MavMessage) -> Result<(), FcError> {
        self.link.send(msg)
    }

    fn send_heartbeat_if_due(&mut self) -> Result<(), FcError> {
        if self.last_heartbeat_sent.elapsed() >= Duration::from_secs(1) {
            self.last_heartbeat_sent = Instant::now();
            let hb = HEARTBEAT_DATA { custom_mode: 0, mavtype: MavType::MAV_TYPE_GCS, autopilot: MavAutopilot::MAV_AUTOPILOT_INVALID, base_mode: MavModeFlag::empty(), system_status: MavState::MAV_STATE_ACTIVE, mavlink_version: 3 };
            self.link.send(&MavMessage::HEARTBEAT(hb))?;
        }
        Ok(())
    }

    /// Receive one frame (or time out), updating heartbeat / param / text state.
    /// Frames from other systems (e.g. our own echo, a companion) are dropped.
    pub fn pump(&mut self, timeout: Duration) -> Result<Option<MavMessage>, FcError> {
        self.send_heartbeat_if_due()?;
        let Some((hdr, msg)): Option<Frame> = self.link.recv(timeout)? else { return Ok(None) };
        if hdr.system_id == GCS_SYSTEM_ID && hdr.component_id == GCS_COMPONENT_ID {
            return Ok(None);
        }
        match &msg {
            MavMessage::HEARTBEAT(h) => {
                if h.autopilot != MavAutopilot::MAV_AUTOPILOT_INVALID && h.mavtype != MavType::MAV_TYPE_GCS {
                    if self.last_heartbeat.is_none() {
                        self.target = (hdr.system_id, hdr.component_id);
                    }
                    if (hdr.system_id, hdr.component_id) == self.target {
                        self.last_heartbeat = Some((Instant::now(), h.clone()));
                    }
                }
            }
            MavMessage::PARAM_VALUE(p) => {
                self.store.insert(p);
            }
            MavMessage::AUTOPILOT_VERSION(v) => self.version = Some(v.clone()),
            MavMessage::STATUSTEXT(t) => {
                let raw: [u8; 50] = t.text.into();
                let end = raw.iter().position(|&b| b == 0).unwrap_or(50);
                let s = String::from_utf8_lossy(&raw[..end]).to_string();
                if s.starts_with("ArduCopter V") || s.starts_with("ArduPlane V") || s.starts_with("ArduRover V") {
                    self.banner = Some(s.clone());
                }
                self.statustext.push(s);
                if self.statustext.len() > 200 {
                    self.statustext.remove(0);
                }
            }
            _ => {}
        }
        Ok(Some(msg))
    }

    /// Pump until `pred` returns `Some` or `timeout` elapses.
    pub fn wait_for<T>(&mut self, timeout: Duration, mut pred: impl FnMut(&MavMessage) -> Option<T>) -> Result<Option<T>, FcError> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            if let Some(m) = self.pump((deadline - now).min(Duration::from_millis(100)))? {
                if let Some(t) = pred(&m) {
                    return Ok(Some(t));
                }
            }
        }
    }

    fn wait_heartbeat(&mut self, timeout: Duration) -> Result<(), FcError> {
        let before = self.last_heartbeat.as_ref().map(|h| h.0);
        let got = self.wait_for(timeout, |_| None::<()>).map(|_| ())?;
        let _ = got;
        match self.last_heartbeat.as_ref().map(|h| h.0) {
            Some(t) if Some(t) != before => Ok(()),
            _ => Err(FcError::Timeout(format!("no MAVLink heartbeat from an autopilot on {} within {:?}", self.address, timeout))),
        }
    }

    pub fn heartbeat_age(&self) -> Option<f32> {
        self.last_heartbeat.as_ref().map(|h| h.0.elapsed().as_secs_f32())
    }

    pub fn armed(&self) -> bool {
        // MAV_MODE_FLAG_SAFETY_ARMED (128) in HEARTBEAT.base_mode
        self.last_heartbeat.as_ref().map(|h| h.1.base_mode.contains(MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED)).unwrap_or(false)
    }

    fn request_version(&mut self) {
        // MAV_CMD_REQUEST_AUTOPILOT_CAPABILITIES param1=1 → AUTOPILOT_VERSION; MAV_CMD_DO_SEND_BANNER → STATUSTEXT "ArduCopter V4.x.y (hash)"
        // MAV_CMD_REQUEST_MESSAGE(AUTOPILOT_VERSION=148) is the current form; ArduPilot also still
        // accepts the deprecated MAV_CMD_REQUEST_AUTOPILOT_CAPABILITIES (GCS_Common.cpp).
        let _ = self.command(MavCmd::MAV_CMD_REQUEST_MESSAGE, [148.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let _ = self.command(MavCmd::MAV_CMD_DO_SEND_BANNER, [0.0; 7]);
        let _ = self.wait_for(Duration::from_millis(1500), |m| matches!(m, MavMessage::AUTOPILOT_VERSION(_)).then_some(()));
    }

    pub fn firmware(&self) -> Firmware {
        let ver = self
            .version
            .as_ref()
            .map(|v| fw_version_string(v.flight_sw_version))
            .or_else(|| self.banner.as_ref().and_then(|b| b.split_whitespace().nth(1).map(|v| v.trim_start_matches('V').to_string())))
            .unwrap_or_default();
        let mavtype = self.last_heartbeat.as_ref().map(|h| h.1.mavtype);
        let is_apm = self.last_heartbeat.as_ref().map(|h| h.1.autopilot == MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA).unwrap_or(false);
        match (is_apm, mavtype) {
            (true, Some(t)) if is_copter(t) => Firmware::ArduCopter { version: ver },
            (true, Some(t)) => Firmware::Unknown { product: format!("ArduPilot {t:?} {ver}") },
            _ => Firmware::Unknown { product: format!("MAVLink {ver}") },
        }
    }

    /// COMMAND_LONG with ACK wait, 3 attempts (ArduPilot ACKs every command it handles).
    pub fn command(&mut self, cmd: MavCmd, p: [f32; 7]) -> Result<MavResult, FcError> {
        let (sys, comp) = self.target;
        for attempt in 0..RETRIES {
            self.send(&MavMessage::COMMAND_LONG(COMMAND_LONG_DATA { param1: p[0], param2: p[1], param3: p[2], param4: p[3], param5: p[4], param6: p[5], param7: p[6], command: cmd, target_system: sys, target_component: comp, confirmation: attempt as u8 }))?;
            if let Some(r) = self.wait_for(ACK_TIMEOUT, |m| match m {
                MavMessage::COMMAND_ACK(a) if a.command == cmd => Some(a.result),
                _ => None,
            })? {
                return Ok(r);
            }
        }
        Err(FcError::Timeout(format!("no COMMAND_ACK for {cmd:?}")))
    }

    /// Full parameter list with retries of missing indices via PARAM_REQUEST_READ.
    pub fn fetch_all_params(&mut self, mut progress: impl FnMut(usize, usize)) -> Result<&ParamStore, FcError> {
        let (sys, comp) = self.target;
        self.store = ParamStore::default();
        self.send(&MavMessage::PARAM_REQUEST_LIST(PARAM_REQUEST_LIST_DATA { target_system: sys, target_component: comp }))?;
        let mut idle = Instant::now();
        let mut last_n = 0;
        loop {
            if let Some(MavMessage::PARAM_VALUE(_)) = self.pump(Duration::from_millis(100))? {
                idle = Instant::now();
            }
            let n = self.store.params.len();
            if n != last_n {
                last_n = n;
                progress(n, self.store.count.unwrap_or(0) as usize);
            }
            if self.store.is_complete() {
                break;
            }
            if idle.elapsed() > PARAM_LIST_IDLE {
                break;
            }
        }
        if self.store.count.is_none() {
            return Err(FcError::Timeout("no PARAM_VALUE received for PARAM_REQUEST_LIST".into()));
        }
        for _round in 0..RETRIES {
            let missing = self.store.missing_indices();
            if missing.is_empty() {
                break;
            }
            for idx in missing {
                self.send(&MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA { param_index: idx as i16, target_system: sys, target_component: comp, param_id: param_id("") }))?;
                self.wait_for(PARAM_READ_TIMEOUT, |m| match m {
                    MavMessage::PARAM_VALUE(p) if p.param_index == idx => Some(()),
                    _ => None,
                })?;
            }
            progress(self.store.params.len(), self.store.count.unwrap_or(0) as usize);
        }
        if !self.store.is_complete() {
            return Err(FcError::Timeout(format!("parameter list incomplete: {}/{}", self.store.params.len(), self.store.count.unwrap_or(0))));
        }
        Ok(&self.store)
    }

    /// PARAM_REQUEST_READ by name (index −1). `None` when the FC reports it does not exist (NaN reply / silence).
    pub fn read_param(&mut self, name: &str) -> Result<Option<f32>, FcError> {
        let (sys, comp) = self.target;
        for _ in 0..RETRIES {
            self.send(&MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA { param_index: -1, target_system: sys, target_component: comp, param_id: param_id(name) }))?;
            let got = self.wait_for(PARAM_READ_TIMEOUT, |m| match m {
                MavMessage::PARAM_VALUE(p) if name_of(&p.param_id) == name => Some(p.param_value),
                _ => None,
            })?;
            if let Some(v) = got {
                return Ok(if v.is_nan() { None } else { Some(v) });
            }
        }
        Ok(None)
    }

    /// Typed PARAM_SET + verification. The FC echoes PARAM_VALUE from
    /// `AP_Param::save_sync` when the value changed; otherwise we read it back.
    pub fn set_param(&mut self, name: &str, value: f32) -> Result<ApplyOutcome, FcError> {
        let (sys, comp) = self.target;
        let ptype = match self.store.get(name) {
            Some(p) => p.ptype,
            None => {
                // Unknown to us (e.g. INS_HNTCH_* appearing after enable): fetch it first for the type.
                if self.read_param(name)?.is_none() {
                    return Ok(ApplyOutcome { param: name.into(), wanted: format!("{value}"), read_back: None, ok: false, via: "mavlink".into() });
                }
                self.store.get(name).map(|p| p.ptype).unwrap_or(MavParamType::MAV_PARAM_TYPE_REAL32)
            }
        };
        let wanted = if crate::params::is_integer_type(ptype) { value.round() } else { value };
        let mut read_back = None;
        for _ in 0..RETRIES {
            self.send(&MavMessage::PARAM_SET(PARAM_SET_DATA { param_value: wanted, target_system: sys, target_component: comp, param_id: param_id(name), param_type: ptype }))?;
            let echoed = self.wait_for(PARAM_READ_TIMEOUT, |m| match m {
                MavMessage::PARAM_VALUE(p) if name_of(&p.param_id) == name => Some(p.param_value),
                _ => None,
            })?;
            let got = match echoed {
                Some(v) => Some(v),
                None => self.read_param(name)?,
            };
            read_back = got;
            if let Some(v) = got {
                if values_match(ptype, wanted, v) {
                    break;
                }
            }
        }
        let ok = read_back.map(|v| values_match(ptype, wanted, v)).unwrap_or(false);
        Ok(ApplyOutcome { param: name.into(), wanted: crate::params::fmt_value(ptype, wanted), read_back: read_back.map(|v| crate::params::fmt_value(ptype, v)), ok, via: "mavlink".into() })
    }

    /// MAV_CMD_PREFLIGHT_REBOOT_SHUTDOWN param1 = 1 (REBOOT_SHUTDOWN_ACTION_REBOOT).
    /// ArduPilot refuses with MAV_RESULT_FAILED while armed (GCS_Common.cpp
    /// handle_preflight_reboot) — we never send the 20190226 force magic.
    pub fn reboot(&mut self) -> Result<(), FcError> {
        if self.armed() {
            return Err(FcError::Refused("armed".into()));
        }
        match self.command(MavCmd::MAV_CMD_PREFLIGHT_REBOOT_SHUTDOWN, [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])? {
            MavResult::MAV_RESULT_ACCEPTED => Ok(()),
            r => Err(FcError::Refused(format!("reboot: {r:?}"))),
        }
    }

    fn status_from_store(&self) -> FcStatus {
        let g = |n: &str| self.store.value(n);
        let bitmask = g("LOG_BITMASK").map(|v| v as u32);
        let loop_rate = g("SCHED_LOOP_RATE").map(|v| v as f64);
        let pid_fast = bitmask.map(|m| m & (LOG_BIT_ATTITUDE_FAST | LOG_BIT_PID) == (LOG_BIT_ATTITUDE_FAST | LOG_BIT_PID));
        let batch = match (bitmask, g("INS_LOG_BAT_MASK")) {
            (Some(m), Some(mask)) => Some(m & LOG_BIT_IMU_RAW != 0 && mask != 0.0),
            _ => None,
        };
        FcStatus {
            connected: true,
            port: Some(self.address.clone()),
            kind: Some(FcKind::Mavlink),
            firmware: Some(self.firmware()),
            armed: self.armed(),
            heartbeat_age_s: self.heartbeat_age().unwrap_or(99.0),
            tune: (!self.store.params.is_empty()).then(|| Tune::Ap(self.store.tune())),
            log_rate_hz: match (pid_fast, loop_rate) {
                (Some(true), Some(l)) => Some(l),
                (Some(false), _) => Some(10.0),
                _ => None,
            },
            debug_mode: None,
            storage_free_bytes: None,
            pid_logging_enabled: pid_fast,
            raw_gyro_logging_enabled: batch,
            snapshot_taken: false,
            log_bitmask: bitmask,
            batch_configured: batch,
            loop_rate_hz: loop_rate,
            autotune_axes: g("AUTOTUNE_AXES").map(|v| v as u8),
        }
    }
}

impl FlightController for MavClient {
    fn kind(&self) -> FcKind {
        FcKind::Mavlink
    }

    fn poll(&mut self) -> Result<FcStatus, FcError> {
        // drain whatever arrived, then make sure a heartbeat is at most ~1.5 s old
        let _ = self.wait_for(Duration::from_millis(50), |_| None::<()>);
        if self.heartbeat_age().map(|a| a > 1.5).unwrap_or(true) {
            let _ = self.wait_for(Duration::from_millis(1200), |m| matches!(m, MavMessage::HEARTBEAT(_)).then_some(()));
        }
        let mut s = self.status_from_store();
        s.connected = self.heartbeat_age().map(|a| a < 5.0).unwrap_or(false);
        Ok(s)
    }

    fn full_status(&mut self) -> Result<FcStatus, FcError> {
        if !self.store.is_complete() {
            self.fetch_all_params(|_, _| {})?;
        }
        Ok(self.status_from_store())
    }

    fn read_tune(&mut self) -> Result<Tune, FcError> {
        if !self.store.is_complete() {
            self.fetch_all_params(|_, _| {})?;
        }
        Ok(Tune::Ap(self.store.tune()))
    }

    fn backup(&mut self) -> Result<Backup, FcError> {
        self.fetch_all_params(|_, _| {})?;
        let hdr = format!("PID Tuner parameter backup — {:?} — {} params", self.firmware(), self.store.params.len());
        Ok(Backup { label: "params".into(), ext: "param".into(), bytes: self.store.to_param_text(&hdr).into_bytes() })
    }

    /// LOG_BITMASK |= ATTITUDE_FAST | PID | IMU_RAW (ArduCopter/defines.h MASK_LOG_*) and
    /// the IMU batch sampler (INS_LOG_BAT_MASK = 1 → reboot required, OPT = 4 pre+post,
    /// CNT = 1024, LGIN = 20 ms). Values not already correct are written and verified.
    fn preflight_fix(&mut self, fix: PreflightFix) -> Result<ApplyResult, FcError> {
        let PreflightFix::Logging = fix;
        if self.armed() {
            return Err(FcError::Refused("armed".into()));
        }
        if !self.store.is_complete() {
            self.fetch_all_params(|_, _| {})?;
        }
        let cur_mask = self.store.value("LOG_BITMASK").unwrap_or(0.0) as u32;
        let want_mask = cur_mask | LOG_BIT_ATTITUDE_FAST | LOG_BIT_PID | LOG_BIT_IMU_RAW;
        let mut plan: Vec<(&str, f32)> = vec![("LOG_BITMASK", want_mask as f32), ("INS_LOG_BAT_MASK", 1.0), ("INS_LOG_BAT_OPT", (INS_LOG_BAT_OPT_PRE_POST) as f32), ("INS_LOG_BAT_CNT", 1024.0), ("INS_LOG_BAT_LGIN", 20.0)];
        plan.retain(|(n, v)| self.store.value(n).map(|c| !values_match(self.store.get(n).unwrap().ptype, *v, c)).unwrap_or(true));
        let mut outcomes = Vec::new();
        let mut need_reboot = false;
        for (n, v) in plan {
            let o = self.set_param(n, v)?;
            if o.ok && n == "INS_LOG_BAT_MASK" {
                need_reboot = true;
            }
            outcomes.push(o);
        }
        let verified = outcomes.iter().all(|o| o.ok);
        let mut rebooted = false;
        if verified && need_reboot {
            self.reboot_and_reconnect()?;
            rebooted = true;
        }
        Ok(ApplyResult { outcomes, verified, rebooted })
    }

    fn apply(&mut self, recs: &[Recommendation]) -> Result<ApplyResult, FcError> {
        if self.armed() {
            return Err(FcError::Refused("flight controller is armed".into()));
        }
        if !self.store.is_complete() {
            self.fetch_all_params(|_, _| {})?;
        }
        let accepted: Vec<&Recommendation> = recs.iter().filter(|r| r.accepted).collect();
        // Reboot-gated params (INS_HNTCH_ENABLE) first: their sub-params only exist afterwards.
        let (gate, rest): (Vec<&Recommendation>, Vec<&Recommendation>) = accepted.iter().partition(|r| r.param.name().ends_with("_ENABLE"));
        let mut outcomes = Vec::new();
        let mut rebooted = false;
        for r in &gate {
            let o = self.set_param(r.param.name(), r.new.as_f64() as f32)?;
            outcomes.push(o);
        }
        if gate.iter().any(|r| r.requires_reboot) && outcomes.iter().all(|o| o.ok) {
            self.reboot_and_reconnect()?;
            rebooted = true;
        }
        let mut any_reboot = false;
        for r in &rest {
            let o = self.set_param(r.param.name(), r.new.as_f64() as f32)?;
            any_reboot |= r.requires_reboot && o.ok;
            outcomes.push(o);
        }
        if any_reboot {
            self.reboot_and_reconnect()?;
            rebooted = true;
        }
        if rebooted {
            // verify everything survived the reboot (values are stored in EEPROM/flash on save)
            for o in outcomes.iter_mut() {
                if let Some(v) = self.store.value(&o.param) {
                    let ptype = self.store.get(&o.param).map(|p| p.ptype).unwrap_or(MavParamType::MAV_PARAM_TYPE_REAL32);
                    let wanted: f32 = o.wanted.parse().unwrap_or(f32::NAN);
                    o.read_back = Some(crate::params::fmt_value(ptype, v));
                    o.ok = values_match(ptype, wanted, v);
                }
            }
        }
        let verified = outcomes.iter().all(|o| o.ok);
        Ok(ApplyResult { outcomes, verified, rebooted })
    }

    fn reboot_and_reconnect(&mut self) -> Result<(), FcError> {
        self.reboot()?;
        let Some(reopen) = self.reopen.take() else {
            // in-memory link (tests): the mock restarts itself
            self.last_heartbeat = None;
            self.wait_heartbeat(Duration::from_secs(5))?;
            self.fetch_all_params(|_, _| {})?;
            return Ok(());
        };
        std::thread::sleep(Duration::from_millis(1500));
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut last_err = None;
        while Instant::now() < deadline {
            match reopen(&self.address) {
                Ok(l) => {
                    self.link = l;
                    self.last_heartbeat = None;
                    if self.wait_heartbeat(Duration::from_secs(5)).is_ok() {
                        self.reopen = Some(reopen);
                        self.fetch_all_params(|_, _| {})?;
                        return Ok(());
                    }
                }
                Err(e) => last_err = Some(e),
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        self.reopen = Some(reopen);
        Err(FcError::Timeout(format!("flight controller did not come back after reboot ({last_err:?})")))
    }

    fn list_logs(&mut self) -> Result<Vec<LogEntry>, FcError> {
        logs::list_logs(self)
    }

    fn download_log(&mut self, id: Option<u32>, progress: ProgressFn<'_>, cancel: &AtomicBool) -> Result<Vec<u8>, FcError> {
        let list = logs::list_logs(self)?;
        let entry = match id {
            Some(i) => list.iter().find(|e| e.id == i).cloned(),
            None => list.last().cloned(),
        }
        .ok_or_else(|| FcError::Other("no such log".into()))?;
        logs::download(self, entry.id, entry.size, progress, cancel)
    }

    fn export_text(&self, recs: &[Recommendation]) -> String {
        export_text_for(Some(&self.firmware()), recs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn firmware_version_decodes_like_send_autopilot_version() {
        assert_eq!(fw_version_string(4 << 24 | 5 << 16 | 5 << 8 | 255), "4.5.5");
        assert_eq!(fw_version_string(4 << 24 | 6 << 16 | 0 << 8 | 192), "4.6.0-rc");
    }
}
