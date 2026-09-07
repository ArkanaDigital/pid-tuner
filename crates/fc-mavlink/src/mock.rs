//! In-memory ArduPilot that answers the MAVLink subset we use, following the
//! handler semantics quoted in `params.rs` / `logs.rs`. Used by the tests and
//! by the app's `--mock` mode.

use crate::apm::*;
use crate::link::{Frame, Link};
use crate::params::{name_of, param_id};
use domain::fc::FcError;
use mavlink::MavHeader;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct MockParam {
    pub value: f32,
    pub ptype: MavParamType,
    pub read_only: bool,
}

#[derive(Debug, Default)]
pub struct MockState {
    pub params: BTreeMap<String, MockParam>,
    pub logs: Vec<Vec<u8>>,
    pub armed: bool,
    /// Indices silently dropped (once) during PARAM_REQUEST_LIST — simulates the 5/tick overflow.
    pub drop_list_indices: HashSet<u16>,
    /// LOG_DATA offsets dropped once.
    pub drop_log_offsets: HashSet<u32>,
    pub reboots: u32,
    pub param_sets: Vec<(String, f32, MavParamType)>,
    /// Params that only appear after a reboot when `gate` is set to 1 (INS_HNTCH_*).
    pub gated: Vec<(String, String, MockParam)>,
    pub version: u32,
}

impl MockState {
    pub fn copter_4_5_5() -> Self {
        let mut s = Self { version: 4 << 24 | 5 << 16 | 5 << 8 | 255, ..Default::default() };
        let f = |v: f32| MockParam { value: v, ptype: MavParamType::MAV_PARAM_TYPE_REAL32, read_only: false };
        let i32_ = |v: f32| MockParam { value: v, ptype: MavParamType::MAV_PARAM_TYPE_INT32, read_only: false };
        let i8_ = |v: f32| MockParam { value: v, ptype: MavParamType::MAV_PARAM_TYPE_INT8, read_only: false };
        let i16_ = |v: f32| MockParam { value: v, ptype: MavParamType::MAV_PARAM_TYPE_INT16, read_only: false };
        for (k, v) in [("ATC_RAT_RLL_P", 0.135), ("ATC_RAT_RLL_I", 0.135), ("ATC_RAT_RLL_D", 0.0036), ("ATC_RAT_PIT_P", 0.135), ("ATC_RAT_PIT_I", 0.135), ("ATC_RAT_PIT_D", 0.0036), ("ATC_RAT_YAW_P", 0.18), ("ATC_RAT_YAW_I", 0.018), ("ATC_RAT_YAW_D", 0.0), ("ATC_RAT_RLL_FLTD", 20.0), ("ATC_RAT_RLL_FLTT", 20.0), ("MOT_THST_HOVER", 0.35), ("AUTOTUNE_AGGR", 0.1), ("AUTOTUNE_MIN_D", 0.001)] {
            s.params.insert(k.into(), f(v));
        }
        s.params.insert("LOG_BITMASK".into(), i32_(180222.0));
        s.params.insert("SCHED_LOOP_RATE".into(), i16_(400.0));
        s.params.insert("INS_GYRO_FILTER".into(), i8_(40.0));
        s.params.insert("INS_LOG_BAT_MASK".into(), i8_(0.0));
        s.params.insert("INS_LOG_BAT_OPT".into(), i16_(0.0));
        s.params.insert("INS_LOG_BAT_CNT".into(), i16_(1024.0));
        s.params.insert("INS_LOG_BAT_LGIN".into(), i8_(20.0));
        s.params.insert("INS_HNTCH_ENABLE".into(), i8_(0.0));
        s.params.insert("AUTOTUNE_AXES".into(), i8_(7.0));
        s.params.insert("FORMAT_VERSION".into(), MockParam { value: 120.0, ptype: MavParamType::MAV_PARAM_TYPE_INT16, read_only: true });
        for (n, v) in [("INS_HNTCH_MODE", 1.0), ("INS_HNTCH_FREQ", 80.0), ("INS_HNTCH_BW", 40.0), ("INS_HNTCH_ATT", 40.0), ("INS_HNTCH_REF", 0.0), ("INS_HNTCH_HMNCS", 3.0)] {
            s.gated.push(("INS_HNTCH_ENABLE".into(), n.into(), f(v)));
        }
        s
    }

    fn visible_names(&self) -> Vec<String> {
        // sorted like AP_Param::first/next enumeration order is irrelevant to clients; use name order
        self.params.keys().cloned().collect()
    }
}

pub struct MockFc {
    pub state: Arc<Mutex<MockState>>,
    out: VecDeque<Frame>,
    last_hb: Instant,
    seq: u8,
    listing: Option<(u16, u16)>,
    sending: Option<(u16, u32, u32)>,
    alive: bool,
}

impl MockFc {
    pub fn new(state: MockState) -> Self {
        Self { state: Arc::new(Mutex::new(state)), out: VecDeque::new(), last_hb: Instant::now() - Duration::from_secs(1), seq: 0, listing: None, sending: None, alive: true }
    }

    pub fn shared(state: Arc<Mutex<MockState>>) -> Self {
        Self { state, out: VecDeque::new(), last_hb: Instant::now() - Duration::from_secs(1), seq: 0, listing: None, sending: None, alive: true }
    }

    fn push(&mut self, m: MavMessage) {
        self.seq = self.seq.wrapping_add(1);
        self.out.push_back((MavHeader { system_id: 1, component_id: 1, sequence: self.seq }, m));
    }

    fn param_value(&self, name: &str, p: &MockParam, index: u16, count: u16) -> MavMessage {
        MavMessage::PARAM_VALUE(PARAM_VALUE_DATA { param_value: p.value, param_count: count, param_index: index, param_id: param_id(name), param_type: p.ptype })
    }

    fn heartbeat(&self) -> MavMessage {
        let armed = self.state.lock().unwrap().armed;
        let mut base = MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED;
        if armed {
            base |= MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED;
        }
        MavMessage::HEARTBEAT(HEARTBEAT_DATA { custom_mode: 2, mavtype: MavType::MAV_TYPE_QUADROTOR, autopilot: MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA, base_mode: base, system_status: MavState::MAV_STATE_STANDBY, mavlink_version: 3 })
    }

    fn handle(&mut self, msg: &MavMessage) {
        match msg {
            MavMessage::PARAM_REQUEST_LIST(_) => {
                let (names, count, drops) = {
                    let s = self.state.lock().unwrap();
                    (s.visible_names(), s.params.len() as u16, s.drop_list_indices.clone())
                };
                self.state.lock().unwrap().drop_list_indices.clear();
                for (i, n) in names.iter().enumerate() {
                    if drops.contains(&(i as u16)) {
                        continue;
                    }
                    let p = self.state.lock().unwrap().params[n].clone();
                    let m = self.param_value(n, &p, i as u16, count);
                    self.push(m);
                }
            }
            MavMessage::PARAM_REQUEST_READ(r) => {
                let s = self.state.lock().unwrap();
                let names = s.visible_names();
                let count = names.len() as u16;
                let found = if r.param_index >= 0 {
                    names.get(r.param_index as usize).map(|n| (n.clone(), s.params[n].clone(), r.param_index as u16))
                } else {
                    let n = name_of(&r.param_id);
                    s.params.get(&n).map(|p| (n.clone(), p.clone(), u16::MAX))
                };
                drop(s);
                match found {
                    Some((n, p, idx)) => {
                        let m = self.param_value(&n, &p, idx, count);
                        self.push(m)
                    }
                    None => {
                        // GCS_Param.cpp: unknown → value NaN, index echoed
                        let n = name_of(&r.param_id);
                        self.push(MavMessage::PARAM_VALUE(PARAM_VALUE_DATA { param_value: f32::NAN, param_count: count, param_index: r.param_index as u16, param_id: param_id(&n), param_type: MavParamType::MAV_PARAM_TYPE_REAL32 }));
                    }
                }
            }
            MavMessage::PARAM_SET(ps) => {
                let n = name_of(&ps.param_id);
                let mut s = self.state.lock().unwrap();
                let count = s.params.len() as u16;
                s.param_sets.push((n.clone(), ps.param_value, ps.param_type));
                let Some(p) = s.params.get_mut(&n) else { return }; // unknown: silence (PARAM_ERROR in new firmware)
                if p.read_only {
                    let p = p.clone();
                    drop(s);
                    let m = self.param_value(&n, &p, u16::MAX, count);
                    self.push(m); // echoes the OLD value
                    return;
                }
                let changed = p.value != ps.param_value;
                // set_float: integer types truncate toward the stored type
                p.value = match ps.param_type {
                    MavParamType::MAV_PARAM_TYPE_REAL32 => ps.param_value,
                    _ => ps.param_value.round(),
                };
                let p2 = p.clone();
                drop(s);
                if changed {
                    // AP_Param::save_sync → GCS_SEND_PARAM (only when value changed)
                    let m = self.param_value(&n, &p2, u16::MAX, count);
                    self.push(m);
                }
            }
            MavMessage::COMMAND_LONG(c) => {
                let result = match c.command {
                    MavCmd::MAV_CMD_PREFLIGHT_REBOOT_SHUTDOWN => {
                        let mut s = self.state.lock().unwrap();
                        if s.armed && c.param6 as u32 != 20190226 {
                            MavResult::MAV_RESULT_FAILED
                        } else if c.param1 == 1.0 || c.param1 == 3.0 {
                            s.reboots += 1;
                            // gated params appear after reboot
                            let gated = s.gated.clone();
                            for (gate, name, p) in gated {
                                if s.params.get(&gate).map(|g| g.value >= 0.5).unwrap_or(false) {
                                    s.params.entry(name).or_insert(p);
                                }
                            }
                            self.alive = false;
                            MavResult::MAV_RESULT_ACCEPTED
                        } else {
                            MavResult::MAV_RESULT_UNSUPPORTED
                        }
                    }
                    MavCmd::MAV_CMD_REQUEST_MESSAGE if c.param1 == 148.0 => {
                        let v = self.state.lock().unwrap().version;
                        self.push(MavMessage::AUTOPILOT_VERSION(AUTOPILOT_VERSION_DATA { flight_sw_version: v, ..Default::default() }));
                        MavResult::MAV_RESULT_ACCEPTED
                    }
                    MavCmd::MAV_CMD_DO_SEND_BANNER => {
                        let mut t = [0u8; 50];
                        for (i, b) in b"ArduCopter V4.5.5 (deadbeef)".iter().enumerate() {
                            t[i] = *b;
                        }
                        self.push(MavMessage::STATUSTEXT(STATUSTEXT_DATA { severity: MavSeverity::MAV_SEVERITY_INFO, text: t.into(), ..Default::default() }));
                        MavResult::MAV_RESULT_ACCEPTED
                    }
                    _ => MavResult::MAV_RESULT_UNSUPPORTED,
                };
                self.push(MavMessage::COMMAND_ACK(COMMAND_ACK_DATA { command: c.command, result, ..Default::default() }));
                if !self.alive {
                    // reboot: come back 100 ms later as a fresh instance
                    std::thread::sleep(Duration::from_millis(100));
                    self.alive = true;
                    self.last_hb = Instant::now() - Duration::from_secs(1);
                }
            }
            MavMessage::LOG_REQUEST_LIST(r) => {
                let n = self.state.lock().unwrap().logs.len() as u16;
                if n == 0 {
                    self.push(MavMessage::LOG_ENTRY(LOG_ENTRY_DATA { time_utc: 0, size: 0, id: 0, num_logs: 0, last_log_num: 0 }));
                } else {
                    let start = r.start.max(1);
                    let end = r.end.min(n);
                    self.listing = Some((start, end));
                }
            }
            MavMessage::LOG_REQUEST_DATA(r) => {
                let s = self.state.lock().unwrap();
                let n = s.logs.len() as u16;
                if r.id < 1 || r.id > n {
                    return; // silently cancelled
                }
                let size = s.logs[r.id as usize - 1].len() as u32;
                drop(s);
                let remaining = if r.ofs >= size { 0 } else { (size - r.ofs).min(r.count) };
                self.sending = Some((r.id, r.ofs, remaining));
            }
            MavMessage::LOG_REQUEST_END(_) => self.sending = None,
            _ => {}
        }
    }

    /// One scheduler tick: a listing entry or a burst of LOG_DATA.
    fn tick(&mut self) {
        if let Some((next, last)) = self.listing {
            let (n, size) = {
                let s = self.state.lock().unwrap();
                (s.logs.len() as u16, s.logs[next as usize - 1].len() as u32)
            };
            self.push(MavMessage::LOG_ENTRY(LOG_ENTRY_DATA { time_utc: 1_700_000_000 + next as u32, size, id: next, num_logs: n, last_log_num: n }));
            self.listing = if next >= last { None } else { Some((next + 1, last)) };
        }
        if let Some((id, ofs, remaining)) = self.sending {
            let mut ofs = ofs;
            let mut remaining = remaining;
            for _ in 0..40 {
                // SITL sends 40 packets per call
                if remaining == 0 {
                    self.sending = None;
                    break;
                }
                let len = remaining.min(90);
                let (chunk, drop) = {
                    let mut s = self.state.lock().unwrap();
                    let log = &s.logs[id as usize - 1];
                    let c = log[ofs as usize..(ofs + len) as usize].to_vec();
                    let d = s.drop_log_offsets.remove(&ofs);
                    (c, d)
                };
                let mut data = [0u8; 90];
                data[..len as usize].copy_from_slice(&chunk);
                if !drop {
                    self.push(MavMessage::LOG_DATA(LOG_DATA_DATA { ofs, id, count: len as u8, data }));
                }
                ofs += len;
                remaining -= len;
                if len < 90 || remaining == 0 {
                    self.sending = None;
                    break;
                }
                self.sending = Some((id, ofs, remaining));
            }
        }
    }
}

impl Link for MockFc {
    fn send(&mut self, msg: &MavMessage) -> Result<(), FcError> {
        self.handle(msg);
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, FcError> {
        if self.last_hb.elapsed() >= Duration::from_millis(200) {
            self.last_hb = Instant::now();
            let hb = self.heartbeat();
            self.push(hb);
        }
        if self.out.is_empty() {
            self.tick();
        }
        if let Some(f) = self.out.pop_front() {
            return Ok(Some(f));
        }
        std::thread::sleep(timeout.min(Duration::from_millis(5)));
        Ok(None)
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MavClient;
    use domain::fc::FlightController;
    use domain::{Confidence, Firmware, ParamRef, ParamValue, Recommendation, Tune};
    use std::sync::atomic::AtomicBool;

    fn client(state: MockState) -> (MavClient, Arc<Mutex<MockState>>) {
        let fc = MockFc::new(state);
        let st = fc.state.clone();
        let c = MavClient::with_link(Box::new(fc), "mock", Duration::from_secs(2)).unwrap();
        (c, st)
    }

    #[test]
    fn connect_reads_heartbeat_and_version() {
        let (c, _) = client(MockState::copter_4_5_5());
        assert_eq!(c.firmware(), Firmware::ArduCopter { version: "4.5.5".into() });
        assert!(!c.armed());
        assert_eq!(c.target(), (1, 1));
    }

    #[test]
    fn param_list_retries_dropped_indices() {
        let mut s = MockState::copter_4_5_5();
        s.drop_list_indices = [2u16, 7].into_iter().collect();
        let (mut c, st) = client(s);
        let n = st.lock().unwrap().params.len();
        let store = c.fetch_all_params(|_, _| {}).unwrap();
        assert_eq!(store.params.len(), n);
        assert!(store.is_complete());
        assert_eq!(store.value("LOG_BITMASK"), Some(180222.0));
        assert_eq!(store.get("LOG_BITMASK").unwrap().ptype, MavParamType::MAV_PARAM_TYPE_INT32);
    }

    #[test]
    fn set_param_sends_int_as_float_and_verifies_echo() {
        let (mut c, st) = client(MockState::copter_4_5_5());
        c.fetch_all_params(|_, _| {}).unwrap();
        let o = c.set_param("LOG_BITMASK", 180222.0 + 1.0 + 4096.0 + 524288.0).unwrap();
        assert!(o.ok, "{o:?}");
        assert_eq!(o.read_back.as_deref(), Some("708607"));
        let sets = st.lock().unwrap().param_sets.clone();
        assert_eq!(sets, vec![("LOG_BITMASK".to_string(), 708607.0, MavParamType::MAV_PARAM_TYPE_INT32)]);
        // unchanged value: no echo from save_sync → read-back path still verifies
        let o = c.set_param("LOG_BITMASK", 708607.0).unwrap();
        assert!(o.ok);
    }

    #[test]
    fn read_only_param_reports_mismatch() {
        let (mut c, _) = client(MockState::copter_4_5_5());
        c.fetch_all_params(|_, _| {}).unwrap();
        let o = c.set_param("FORMAT_VERSION", 5.0).unwrap();
        assert!(!o.ok);
        assert_eq!(o.read_back.as_deref(), Some("120"));
        let o = c.set_param("NOPE_PARAM", 1.0).unwrap();
        assert!(!o.ok && o.read_back.is_none());
    }

    #[test]
    fn apply_gates_hntch_enable_behind_reboot_and_verifies_after() {
        let (mut c, st) = client(MockState::copter_4_5_5());
        let rec = |n: &str, v: f32, reboot: bool| Recommendation { id: uuid::Uuid::nil(), param: ParamRef::Ap(n.into()), old: ParamValue::F32(0.0), new: ParamValue::F32(v), reason: String::new(), evidence: vec![], confidence: Confidence::High, requires_reboot: reboot, accepted: true };
        let recs = vec![rec("INS_HNTCH_ENABLE", 1.0, true), rec("INS_HNTCH_FREQ", 120.0, false), rec("INS_HNTCH_REF", 0.35, false), rec("ATC_RAT_RLL_P", 0.15, false)];
        let r = c.apply(&recs).unwrap();
        assert!(r.rebooted);
        assert!(r.verified, "{:?}", r.outcomes);
        assert_eq!(st.lock().unwrap().reboots, 1);
        let s = st.lock().unwrap();
        assert_eq!(s.params["INS_HNTCH_FREQ"].value, 120.0);
        assert_eq!(s.params["INS_HNTCH_ENABLE"].value, 1.0);
        assert!((s.params["ATC_RAT_RLL_P"].value - 0.15).abs() < 1e-7);
        // order: ENABLE was set before FREQ existed
        let names: Vec<&str> = s.param_sets.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(names, vec!["INS_HNTCH_ENABLE", "INS_HNTCH_FREQ", "INS_HNTCH_REF", "ATC_RAT_RLL_P"]);
    }

    #[test]
    fn apply_refused_when_armed_and_reboot_fails_when_armed() {
        let mut s = MockState::copter_4_5_5();
        s.armed = true;
        let (mut c, _) = client(s);
        assert!(c.armed());
        assert!(matches!(c.apply(&[]), Err(domain::fc::FcError::Refused(_))));
        assert!(matches!(c.reboot(), Err(domain::fc::FcError::Refused(_))));
    }

    #[test]
    fn preflight_fix_sets_bits_and_batch_sampler_then_reboots() {
        let (mut c, st) = client(MockState::copter_4_5_5());
        let r = c.preflight_fix(domain::fc::PreflightFix::Logging).unwrap();
        assert!(r.verified, "{:?}", r.outcomes);
        assert!(r.rebooted);
        let s = st.lock().unwrap();
        let m = s.params["LOG_BITMASK"].value as u32;
        assert_eq!(m & (1 | 1 << 12 | 1 << 19), 1 | 1 << 12 | 1 << 19);
        assert_eq!(s.params["INS_LOG_BAT_MASK"].value, 1.0);
        assert_eq!(s.params["INS_LOG_BAT_OPT"].value, 4.0);
        drop(s);
        let st2 = c.full_status().unwrap();
        assert_eq!(st2.batch_configured, Some(true));
        assert_eq!(st2.pid_logging_enabled, Some(true));
        assert_eq!(st2.log_rate_hz, Some(400.0));
        // second run: nothing to do, no reboot
        let r = c.preflight_fix(domain::fc::PreflightFix::Logging).unwrap();
        assert!(r.outcomes.is_empty() && !r.rebooted);
    }

    #[test]
    fn log_download_is_byte_identical_with_dropped_chunks() {
        let mut s = MockState::copter_4_5_5();
        let log1: Vec<u8> = (0..(90 * 37 + 17)).map(|i| (i * 7 % 251) as u8).collect();
        let log2: Vec<u8> = (0..(90 * 12)).map(|i| (i * 13 % 253) as u8).collect(); // exact multiple of 90
        s.logs = vec![log1.clone(), log2.clone()];
        s.drop_log_offsets = [90u32 * 3, 90 * 20, 90 * 36].into_iter().collect();
        let (mut c, _) = client(s);
        let list = c.list_logs().unwrap();
        assert_eq!(list.iter().map(|e| (e.id, e.size)).collect::<Vec<_>>(), vec![(1, log1.len() as u64), (2, log2.len() as u64)]);
        let mut last = (0, 0);
        let got = c.download_log(Some(1), &mut |d, t| last = (d, t), &AtomicBool::new(false)).unwrap();
        assert_eq!(got, log1);
        assert_eq!(last, (log1.len() as u64, log1.len() as u64));
        let got = c.download_log(None, &mut |_, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(got, log2);
    }

    #[test]
    fn backup_is_param_text_and_status_reflects_log_bitmask() {
        let (mut c, _) = client(MockState::copter_4_5_5());
        let b = c.backup().unwrap();
        assert_eq!(b.ext, "param");
        let t = String::from_utf8(b.bytes).unwrap();
        assert!(t.contains("LOG_BITMASK,180222\n"));
        assert!(t.contains("ATC_RAT_RLL_P,0.135\n"));
        let st = c.full_status().unwrap();
        assert_eq!(st.log_bitmask, Some(180222));
        assert_eq!(st.pid_logging_enabled, Some(false));
        assert_eq!(st.log_rate_hz, Some(10.0));
        assert_eq!(st.batch_configured, Some(false));
        assert_eq!(st.loop_rate_hz, Some(400.0));
        assert_eq!(st.autotune_axes, Some(7));
        assert!(matches!(st.tune, Some(Tune::Ap(_))));
    }
}
