//! High-level Betaflight client.

use crate::cli::Cli;
use crate::codec::MspError;
use crate::codes::*;
use crate::layouts::*;
use crate::transport::{MspLink, SerialTransport, Transport};
use domain::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct MspClient {
    pub link: MspLink,
    pub identity: Identity,
    pub port: String,
}

pub use domain::fc::{ApplyOutcome, ApplyResult};

/// Everything we can read over MSP, stored as the pre-write snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MspSnapshot {
    pub identity: Identity,
    pub pids: Pids,
    pub advanced: PidAdvanced,
    pub filters: FilterConfig,
    pub simplified: Option<SimplifiedTuning>,
    pub blackbox: Option<BlackboxConfig>,
    pub advanced_config: Option<AdvancedConfig>,
}

fn bf_type(v: u8) -> BfFilterType {
    match v {
        1 => BfFilterType::Biquad,
        2 => BfFilterType::Pt2,
        3 => BfFilterType::Pt3,
        _ => BfFilterType::Pt1,
    }
}
fn type_u8(t: BfFilterType) -> u8 {
    match t {
        BfFilterType::Pt1 => 0,
        BfFilterType::Biquad => 1,
        BfFilterType::Pt2 => 2,
        BfFilterType::Pt3 => 3,
    }
}

impl MspClient {
    pub fn open(port: &str) -> Result<Self, MspError> {
        let t = SerialTransport::open(port, 115_200)?;
        Self::with_transport(Box::new(t), port)
    }

    pub fn with_transport(t: Box<dyn Transport>, port: &str) -> Result<Self, MspError> {
        let mut link = MspLink::new(t);
        let identity = Self::handshake(&mut link)?;
        Ok(Self {
            link,
            identity,
            port: port.to_string(),
        })
    }

    fn handshake(link: &mut MspLink) -> Result<Identity, MspError> {
        let (proto, api) = parse_api_version(&link.request(MSP_API_VERSION, &[])?)?;
        let variant = parse_fc_variant(&link.request(MSP_FC_VARIANT, &[])?)?;
        if variant != "BTFL" {
            return Err(MspError::Protocol(format!(
                "not a Betaflight FC (variant {variant})"
            )));
        }
        let version = parse_fc_version(&link.request(MSP_FC_VERSION, &[])?)?;
        let (board_id, target_name, board_name) = link
            .request(MSP_BOARD_INFO, &[])
            .ok()
            .and_then(|p| parse_board_info(&p).ok())
            .unwrap_or_default();
        Ok(Identity {
            msp_protocol: proto,
            api,
            variant,
            version,
            board_id,
            target_name,
            board_name,
        })
    }

    pub fn api(&self) -> ApiVersion {
        self.identity.api
    }

    pub fn firmware(&self) -> Firmware {
        Firmware::Betaflight {
            version: self.identity.version.clone(),
            api: (self.identity.api.major, self.identity.api.minor),
        }
    }

    pub fn status(&mut self) -> Result<StatusEx, MspError> {
        StatusEx::parse(&self.link.request(MSP_STATUS_EX, &[])?)
    }

    pub fn read_pids(&mut self) -> Result<Pids, MspError> {
        Pids::parse(&self.link.request(MSP_PID, &[])?)
    }
    pub fn read_pid_advanced(&mut self) -> Result<PidAdvanced, MspError> {
        PidAdvanced::parse(&self.link.request(MSP_PID_ADVANCED, &[])?, self.api())
    }
    pub fn read_filters(&mut self) -> Result<FilterConfig, MspError> {
        FilterConfig::parse(&self.link.request(MSP_FILTER_CONFIG, &[])?, self.api())
    }
    pub fn read_simplified(&mut self) -> Result<Option<SimplifiedTuning>, MspError> {
        if !self.api().at_least(1, 44) {
            return Ok(None);
        }
        match self.link.request(MSP_SIMPLIFIED_TUNING, &[]) {
            Ok(p) => Ok(Some(SimplifiedTuning::parse(&p)?)),
            Err(MspError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn read_blackbox(&mut self) -> Result<Option<BlackboxConfig>, MspError> {
        match self.link.request(MSP_BLACKBOX_CONFIG, &[]) {
            Ok(p) => Ok(Some(BlackboxConfig::parse(&p, self.api())?)),
            Err(MspError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn read_advanced_config(&mut self) -> Result<Option<AdvancedConfig>, MspError> {
        match self.link.request(MSP_ADVANCED_CONFIG, &[]) {
            Ok(p) => Ok(Some(AdvancedConfig::parse(&p)?)),
            Err(MspError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn dataflash_summary(&mut self) -> Result<DataflashSummary, MspError> {
        DataflashSummary::parse(&self.link.request(MSP_DATAFLASH_SUMMARY, &[])?)
    }
    pub fn sdcard_summary(&mut self) -> Result<Option<SdcardSummary>, MspError> {
        match self.link.request(MSP_SDCARD_SUMMARY, &[]) {
            Ok(p) => Ok(Some(SdcardSummary::parse(&p)?)),
            Err(MspError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn snapshot(&mut self) -> Result<MspSnapshot, MspError> {
        Ok(MspSnapshot {
            identity: self.identity.clone(),
            pids: self.read_pids()?,
            advanced: self.read_pid_advanced()?,
            filters: self.read_filters()?,
            simplified: self.read_simplified()?,
            blackbox: self.read_blackbox()?,
            advanced_config: self.read_advanced_config()?,
        })
    }

    /// Effective blackbox logging rate in Hz, if computable.
    pub fn blackbox_rate_hz(&mut self) -> Result<Option<f64>, MspError> {
        let Some(bb) = self.read_blackbox()? else {
            return Ok(None);
        };
        let Some(adv) = self.read_advanced_config()? else {
            return Ok(None);
        };
        let st = self.status()?;
        // cycle time is the PID loop period in µs
        let loop_hz = if st.cycle_time_us > 0 {
            1e6 / st.cycle_time_us as f64
        } else {
            8000.0 / adv.pid_process_denom.max(1) as f64
        };
        let rate = if self.api().at_least(1, 44) {
            loop_hz / bb.sample_divisor() as f64
        } else {
            loop_hz * bb.rate_num.max(1) as f64 / bb.rate_denom.max(1) as f64
        };
        Ok(Some(rate))
    }

    pub fn read_tune(&mut self) -> Result<BfTune, MspError> {
        let snap = self.snapshot()?;
        Ok(tune_from_snapshot(&snap))
    }

    // ---- writes ----------------------------------------------------------

    pub fn write_pids(&mut self, p: &Pids) -> Result<(), MspError> {
        self.link.request(MSP_SET_PID, &p.encode()).map(|_| ())
    }
    pub fn write_pid_advanced(&mut self, a: &PidAdvanced) -> Result<(), MspError> {
        self.link
            .request(MSP_SET_PID_ADVANCED, &a.encode(self.api()))
            .map(|_| ())
    }
    pub fn write_filters(&mut self, f: &FilterConfig) -> Result<(), MspError> {
        self.link
            .request(MSP_SET_FILTER_CONFIG, &f.encode(self.api()))
            .map(|_| ())
    }
    pub fn write_simplified(&mut self, s: &SimplifiedTuning) -> Result<(), MspError> {
        self.link
            .request(MSP_SET_SIMPLIFIED_TUNING, &s.encode())
            .map(|_| ())
    }
    pub fn write_blackbox(&mut self, b: &BlackboxConfig) -> Result<(), MspError> {
        self.link
            .request(MSP_SET_BLACKBOX_CONFIG, &b.encode(self.api()))
            .map(|_| ())
    }
    pub fn write_advanced_config(&mut self, a: &AdvancedConfig) -> Result<(), MspError> {
        self.link
            .request(MSP_SET_ADVANCED_CONFIG, &a.encode())
            .map(|_| ())
    }
    pub fn eeprom_write(&mut self) -> Result<(), MspError> {
        self.link.request(MSP_EEPROM_WRITE, &[]).map(|_| ())
    }
    /// Reboot (firmware). The link is dead afterwards.
    pub fn reboot(&mut self, kind: u8) -> Result<(), MspError> {
        let _ = self.link.request(MSP_SET_REBOOT, &[kind]);
        Ok(())
    }

    /// Download the whole used dataflash region.
    pub fn dataflash_download(
        &mut self,
        mut progress: impl FnMut(u32, u32),
    ) -> Result<Vec<u8>, MspError> {
        let s = self.dataflash_summary()?;
        if !s.supported || !s.ready {
            return Err(MspError::Protocol("dataflash not available".into()));
        }
        let total = s.used_size;
        let mut out = Vec::with_capacity(total as usize);
        let chunk: u16 = 4096;
        let mut addr = 0u32;
        while addr < total {
            let want = chunk.min((total - addr).min(u16::MAX as u32) as u16);
            let c = parse_dataflash_read(
                &self
                    .link
                    .request(MSP_DATAFLASH_READ, &encode_dataflash_read(addr, want))?,
            )?;
            if c.address != addr {
                continue; // stale reply, ask again
            }
            if c.data.is_empty() {
                break;
            }
            addr += c.data.len() as u32;
            out.extend_from_slice(&c.data);
            progress(addr, total);
        }
        Ok(out)
    }

    /// `diff all` via CLI. **Reboots the FC**; reconnect afterwards.
    /// `diff all` via the CLI. Leaving the CLI (`exit`) reboots the FC: the
    /// caller must reconnect (`reconnect`) afterwards.
    pub fn cli_diff_all(&mut self) -> Result<String, MspError> {
        let mut cli = Cli::enter(&mut self.link)?;
        let diff = cli.diff_all()?;
        cli.exit()?;
        Ok(diff)
    }

    /// Apply recommendations: structured MSP writes first, EEPROM save, then
    /// re-read everything and compare. Params without an MSP field go through
    /// the CLI (`set` + `save`, which reboots).
    pub fn apply(&mut self, recs: &[Recommendation]) -> Result<ApplyResult, MspError> {
        let st = self.status()?;
        if st.armed() {
            return Err(MspError::Refused("flight controller is armed".into()));
        }
        let mut pids = self.read_pids()?;
        let mut adv = self.read_pid_advanced()?;
        let mut flt = self.read_filters()?;
        let mut simp = self.read_simplified()?;
        let (mut d_pids, mut d_adv, mut d_flt, mut d_simp) = (false, false, false, false);
        let mut cli_set: Vec<(String, String)> = Vec::new();
        let mut plan: Vec<(String, String, &'static str)> = Vec::new();

        for r in recs.iter().filter(|r| r.accepted) {
            let name = r.param.name().to_string();
            let v = r.new.as_f64();
            let legacy = !self.api().at_least(1, 47);
            let via = if apply_one(
                &name,
                v,
                legacy,
                &mut pids,
                &mut adv,
                &mut flt,
                simp.as_mut(),
                &mut d_pids,
                &mut d_adv,
                &mut d_flt,
                &mut d_simp,
            ) {
                "msp"
            } else {
                cli_set.push((name.clone(), r.new.to_string()));
                "cli"
            };
            plan.push((name, r.new.to_string(), via));
        }

        // Sliders first so the FC does not recompute values we are about to write.
        if d_simp {
            if let Some(s) = &simp {
                self.write_simplified(s)?;
            }
        }
        if d_pids {
            self.write_pids(&pids)?;
        }
        if d_adv {
            self.write_pid_advanced(&adv)?;
        }
        if d_flt {
            self.write_filters(&flt)?;
        }
        if d_simp || d_pids || d_adv || d_flt {
            self.eeprom_write()?;
        }

        // Verify MSP-written values by re-reading.
        let snap = self.snapshot()?;
        let tune = tune_from_snapshot(&snap);
        let mut outcomes = Vec::new();
        let mut all_ok = true;
        for (name, wanted, via) in &plan {
            if *via != "msp" {
                continue;
            }
            let rb = session_lookup(&tune, name);
            let ok = rb
                .map(|x| (x - wanted.parse::<f64>().unwrap_or(f64::NAN)).abs() < 1e-6)
                .unwrap_or(false);
            all_ok &= ok;
            outcomes.push(ApplyOutcome {
                param: name.clone(),
                wanted: wanted.clone(),
                read_back: rb.map(fmt_num),
                ok,
                via: via.to_string(),
            });
        }

        let mut rebooted = false;
        if !cli_set.is_empty() {
            let mut cli = Cli::enter(&mut self.link)?;
            let mut ok_all = true;
            for (n, v) in &cli_set {
                let ok = cli.set(n, v).is_ok();
                ok_all &= ok;
                outcomes.push(ApplyOutcome {
                    param: n.clone(),
                    wanted: v.clone(),
                    read_back: cli.get(n).ok().flatten(),
                    ok,
                    via: "cli".into(),
                });
            }
            cli.save()?;
            rebooted = true;
            all_ok &= ok_all;
        }
        Ok(ApplyResult {
            outcomes,
            verified: all_ok,
            rebooted,
        })
    }
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// Mutate the right struct for a CLI-style parameter name. Returns false if
/// the name has no MSP representation.
#[allow(clippy::too_many_arguments)]
fn apply_one(
    name: &str,
    v: f64,
    legacy: bool,
    pids: &mut Pids,
    adv: &mut PidAdvanced,
    flt: &mut FilterConfig,
    simp: Option<&mut SimplifiedTuning>,
    d_pids: &mut bool,
    d_adv: &mut bool,
    d_flt: &mut bool,
    d_simp: &mut bool,
) -> bool {
    let u8v = v.round().clamp(0.0, 255.0) as u8;
    let u16v = v.round().clamp(0.0, 65535.0) as u16;
    let axis = |n: &str| ["roll", "pitch", "yaw"].iter().position(|a| n.ends_with(a));
    if let Some(k) = axis(name) {
        if pids.rows.len() > k {
            if name.starts_with("p_") {
                pids.rows[k][0] = u8v;
                *d_pids = true;
                return true;
            }
            if name.starts_with("i_") {
                pids.rows[k][1] = u8v;
                *d_pids = true;
                return true;
            }
            // MSP_PID row D and PID_ADVANCED "dMax" swap meaning across versions:
            //   ≤ 4.5 (API < 1.47): row D = D Max (CLI d_*),   dMax field = Derivative (CLI d_min_*)
            //   2025.12+:           row D = Derivative (d_*),  dMax field = D Max (d_max_*)
            let is_adv_field = if legacy {
                name.starts_with("d_min_")
            } else {
                name.starts_with("d_max_")
            };
            let is_row_d = name.starts_with("d_")
                && !name.starts_with("d_max_")
                && !name.starts_with("d_min_");
            if is_adv_field {
                match k {
                    0 => adv.d_max_roll = u8v,
                    1 => adv.d_max_pitch = u8v,
                    _ => adv.d_max_yaw = u8v,
                }
                *d_adv = true;
                return true;
            }
            if is_row_d {
                pids.rows[k][2] = u8v;
                *d_pids = true;
                return true;
            }
            if name.starts_with("d_max_") || name.starts_with("d_min_") {
                return false; // name from the other naming scheme: fall through to CLI
            }
            if name.starts_with("f_") {
                match k {
                    0 => adv.feedforward_roll = u16v,
                    1 => adv.feedforward_pitch = u16v,
                    _ => adv.feedforward_yaw = u16v,
                }
                *d_adv = true;
                return true;
            }
        }
    }
    match name {
        "d_max_gain" => {
            adv.d_max_gain = u8v;
            *d_adv = true;
        }
        "d_max_advance" => {
            adv.d_max_advance = u8v;
            *d_adv = true;
        }
        "anti_gravity_gain" => {
            adv.anti_gravity_gain = u16v;
            *d_adv = true;
        }
        "iterm_relax" => {
            adv.iterm_relax = u8v;
            *d_adv = true;
        }
        "iterm_relax_type" => {
            adv.iterm_relax_type = u8v;
            *d_adv = true;
        }
        "iterm_relax_cutoff" => {
            adv.iterm_relax_cutoff = u8v;
            *d_adv = true;
        }
        "feedforward_smooth_factor" => {
            adv.feedforward_smooth_factor = u8v;
            *d_adv = true;
        }
        "feedforward_jitter_factor" => {
            adv.feedforward_jitter_factor = u8v;
            *d_adv = true;
        }
        "feedforward_boost" => {
            adv.feedforward_boost = u8v;
            *d_adv = true;
        }
        "feedforward_averaging" => {
            adv.feedforward_averaging = u8v;
            *d_adv = true;
        }
        "feedforward_max_rate_limit" => {
            adv.feedforward_max_rate_limit = u8v;
            *d_adv = true;
        }
        "tpa_rate" => {
            adv.tpa_rate = u8v;
            *d_adv = true;
        }
        "tpa_breakpoint" => {
            adv.tpa_breakpoint = u16v;
            *d_adv = true;
        }
        "gyro_lpf1_static_hz" => {
            flt.gyro_lowpass_hz = u16v;
            *d_flt = true;
        }
        "gyro_lpf1_type" => {
            flt.gyro_lowpass_type = u8v;
            *d_flt = true;
        }
        "gyro_lpf1_dyn_min_hz" => {
            flt.gyro_lowpass_dyn_min_hz = u16v;
            *d_flt = true;
        }
        "gyro_lpf1_dyn_max_hz" => {
            flt.gyro_lowpass_dyn_max_hz = u16v;
            *d_flt = true;
        }
        "gyro_lpf2_static_hz" => {
            flt.gyro_lowpass2_hz = u16v;
            *d_flt = true;
        }
        "gyro_lpf2_type" => {
            flt.gyro_lowpass2_type = u8v;
            *d_flt = true;
        }
        "gyro_notch1_hz" => {
            flt.gyro_notch_hz = u16v;
            *d_flt = true;
        }
        "gyro_notch1_cutoff" => {
            flt.gyro_notch_cutoff = u16v;
            *d_flt = true;
        }
        "gyro_notch2_hz" => {
            flt.gyro_notch2_hz = u16v;
            *d_flt = true;
        }
        "gyro_notch2_cutoff" => {
            flt.gyro_notch2_cutoff = u16v;
            *d_flt = true;
        }
        "dterm_lpf1_static_hz" => {
            flt.dterm_lowpass_hz = u16v;
            *d_flt = true;
        }
        "dterm_lpf1_type" => {
            flt.dterm_lowpass_type = u8v;
            *d_flt = true;
        }
        "dterm_lpf1_dyn_min_hz" => {
            flt.dterm_lowpass_dyn_min_hz = u16v;
            *d_flt = true;
        }
        "dterm_lpf1_dyn_max_hz" => {
            flt.dterm_lowpass_dyn_max_hz = u16v;
            *d_flt = true;
        }
        "dterm_lpf2_static_hz" => {
            flt.dterm_lowpass2_hz = u16v;
            *d_flt = true;
        }
        "dterm_lpf2_type" => {
            flt.dterm_lowpass2_type = u8v;
            *d_flt = true;
        }
        "dterm_notch_hz" => {
            flt.dterm_notch_hz = u16v;
            *d_flt = true;
        }
        "dterm_notch_cutoff" => {
            flt.dterm_notch_cutoff = u16v;
            *d_flt = true;
        }
        "dyn_notch_count" => {
            flt.dyn_notch_count = u8v;
            *d_flt = true;
        }
        "dyn_notch_q" => {
            flt.dyn_notch_q = u16v;
            *d_flt = true;
        }
        "dyn_notch_min_hz" => {
            flt.dyn_notch_min_hz = u16v;
            *d_flt = true;
        }
        "dyn_notch_max_hz" => {
            flt.dyn_notch_max_hz = u16v;
            *d_flt = true;
        }
        "rpm_filter_harmonics" => {
            flt.gyro_rpm_notch_harmonics = u8v;
            *d_flt = true;
        }
        "rpm_filter_min_hz" => {
            flt.gyro_rpm_notch_min_hz = u8v;
            *d_flt = true;
        }
        "rpm_filter_q" if flt.has_rpm_ext => {
            flt.gyro_rpm_notch_q = u16v;
            *d_flt = true;
        }
        "rpm_filter_fade_range_hz" if flt.has_rpm_ext => {
            flt.gyro_rpm_notch_fade_range_hz = u16v;
            *d_flt = true;
        }
        "simplified_pids_mode"
        | "simplified_master_multiplier"
        | "simplified_pi_gain"
        | "simplified_i_gain"
        | "simplified_d_gain"
        | "simplified_d_max_gain"
        | "simplified_feedforward_gain"
        | "simplified_pitch_pi_gain"
        | "simplified_pitch_d_gain"
        | "simplified_gyro_filter"
        | "simplified_gyro_filter_multiplier"
        | "simplified_dterm_filter"
        | "simplified_dterm_filter_multiplier" => {
            let Some(s) = simp else { return false };
            match name {
                "simplified_pids_mode" => s.pids_mode = u8v,
                "simplified_master_multiplier" => s.master_multiplier = u8v,
                "simplified_pi_gain" => s.pi_gain = u8v,
                "simplified_i_gain" => s.i_gain = u8v,
                "simplified_d_gain" => s.d_gain = u8v,
                "simplified_d_max_gain" => s.dmax_gain = u8v,
                "simplified_feedforward_gain" => s.feedforward_gain = u8v,
                "simplified_pitch_pi_gain" => s.pitch_pi_gain = u8v,
                "simplified_pitch_d_gain" => s.roll_pitch_ratio = u8v,
                "simplified_gyro_filter" => s.gyro_filter = u8v,
                "simplified_gyro_filter_multiplier" => s.gyro_filter_multiplier = u8v,
                "simplified_dterm_filter" => s.dterm_filter = u8v,
                _ => s.dterm_filter_multiplier = u8v,
            }
            *d_simp = true;
        }
        _ => return false,
    }
    true
}

/// Same name→value lookup the session guards use, on a tune read from MSP.
fn session_lookup(t: &BfTune, name: &str) -> Option<f64> {
    let axis = |n: &str| ["roll", "pitch", "yaw"].iter().position(|a| n.ends_with(a));
    let f = &t.filters;
    let s = &t.simplified;
    Some(match name {
        n if n.starts_with("p_") => t.pids[axis(n)?].p as f64,
        n if n.starts_with("i_") => t.pids[axis(n)?].i as f64,
        n if n.starts_with("d_max_") && axis(n).is_some() => t.pids[axis(n)?].d_max as f64,
        n if n.starts_with("d_min_") && axis(n).is_some() => t.pids[axis(n)?].d as f64,
        n if n.starts_with("d_") && axis(n).is_some() && t.legacy_d_naming() => {
            t.pids[axis(n)?].d_max as f64
        }
        n if n.starts_with("d_") && axis(n).is_some() => t.pids[axis(n)?].d as f64,
        n if n.starts_with("f_") => t.pids[axis(n)?].ff as f64,
        "d_max_gain" => t.d_max_gain as f64,
        "d_max_advance" => t.d_max_advance as f64,
        "anti_gravity_gain" => t.anti_gravity_gain as f64,
        "iterm_relax" => t.iterm_relax as f64,
        "iterm_relax_type" => t.iterm_relax_type as f64,
        "iterm_relax_cutoff" => t.iterm_relax_cutoff as f64,
        "feedforward_smooth_factor" => t.feedforward_smooth_factor as f64,
        "feedforward_jitter_factor" => t.feedforward_jitter_factor as f64,
        "feedforward_boost" => t.feedforward_boost as f64,
        "tpa_rate" => t.tpa_rate as f64,
        "tpa_breakpoint" => t.tpa_breakpoint as f64,
        "gyro_lpf1_static_hz" => f.gyro_lpf1_static_hz as f64,
        "gyro_lpf1_type" => type_u8(f.gyro_lpf1_type) as f64,
        "gyro_lpf1_dyn_min_hz" => f.gyro_lpf1_dyn_min_hz as f64,
        "gyro_lpf1_dyn_max_hz" => f.gyro_lpf1_dyn_max_hz as f64,
        "gyro_lpf2_static_hz" => f.gyro_lpf2_static_hz as f64,
        "gyro_lpf2_type" => type_u8(f.gyro_lpf2_type) as f64,
        "gyro_notch1_hz" => f.gyro_notch1_hz as f64,
        "gyro_notch1_cutoff" => f.gyro_notch1_cutoff as f64,
        "gyro_notch2_hz" => f.gyro_notch2_hz as f64,
        "gyro_notch2_cutoff" => f.gyro_notch2_cutoff as f64,
        "dterm_lpf1_static_hz" => f.dterm_lpf1_static_hz as f64,
        "dterm_lpf1_type" => type_u8(f.dterm_lpf1_type) as f64,
        "dterm_lpf1_dyn_min_hz" => f.dterm_lpf1_dyn_min_hz as f64,
        "dterm_lpf1_dyn_max_hz" => f.dterm_lpf1_dyn_max_hz as f64,
        "dterm_lpf2_static_hz" => f.dterm_lpf2_static_hz as f64,
        "dterm_lpf2_type" => type_u8(f.dterm_lpf2_type) as f64,
        "dterm_notch_hz" => f.dterm_notch_hz as f64,
        "dterm_notch_cutoff" => f.dterm_notch_cutoff as f64,
        "dyn_notch_count" => f.dyn_notch_count as f64,
        "dyn_notch_q" => f.dyn_notch_q as f64,
        "dyn_notch_min_hz" => f.dyn_notch_min_hz as f64,
        "dyn_notch_max_hz" => f.dyn_notch_max_hz as f64,
        "rpm_filter_harmonics" => f.rpm_filter_harmonics as f64,
        "rpm_filter_min_hz" => f.rpm_filter_min_hz as f64,
        "rpm_filter_q" => f.rpm_filter_q as f64,
        "rpm_filter_fade_range_hz" => f.rpm_filter_fade_range_hz as f64,
        "simplified_pids_mode" => s.pids_mode as f64,
        "simplified_master_multiplier" => s.master_multiplier as f64,
        "simplified_pi_gain" => s.pi_gain as f64,
        "simplified_i_gain" => s.i_gain as f64,
        "simplified_d_gain" => s.d_gain as f64,
        "simplified_d_max_gain" => s.d_max_gain as f64,
        "simplified_feedforward_gain" => s.feedforward_gain as f64,
        "simplified_pitch_pi_gain" => s.pitch_pi_gain as f64,
        "simplified_pitch_d_gain" => s.pitch_d_gain as f64,
        "simplified_gyro_filter" => s.gyro_filter as u8 as f64,
        "simplified_gyro_filter_multiplier" => s.gyro_filter_multiplier as f64,
        "simplified_dterm_filter" => s.dterm_filter as u8 as f64,
        "simplified_dterm_filter_multiplier" => s.dterm_filter_multiplier as f64,
        _ => return None,
    })
}

pub fn tune_from_snapshot(s: &MspSnapshot) -> BfTune {
    let mut t = BfTune::default();
    for k in 0..3 {
        if let Some(r) = s.pids.rows.get(k) {
            t.pids[k].p = r[0];
            t.pids[k].i = r[1];
            t.pids[k].d = r[2];
        }
    }
    let a = &s.advanced;
    let legacy = !s.identity.api.at_least(1, 47);
    if legacy {
        // row D was D Max; the "dMax" MSP field carries the Derivative (d_min) value
        for k in 0..3 {
            t.pids[k].d_max = t.pids[k].d;
        }
        t.pids[0].d = a.d_max_roll;
        t.pids[1].d = a.d_max_pitch;
        t.pids[2].d = a.d_max_yaw;
    } else {
        t.pids[0].d_max = a.d_max_roll;
        t.pids[1].d_max = a.d_max_pitch;
        t.pids[2].d_max = a.d_max_yaw;
    }
    t.pids[0].ff = a.feedforward_roll;
    t.pids[1].ff = a.feedforward_pitch;
    t.pids[2].ff = a.feedforward_yaw;
    t.d_max_gain = a.d_max_gain;
    t.d_max_advance = a.d_max_advance;
    t.anti_gravity_gain = a.anti_gravity_gain;
    t.iterm_relax = a.iterm_relax;
    t.iterm_relax_type = a.iterm_relax_type;
    t.iterm_relax_cutoff = a.iterm_relax_cutoff;
    t.feedforward_smooth_factor = a.feedforward_smooth_factor;
    t.feedforward_jitter_factor = a.feedforward_jitter_factor;
    t.feedforward_boost = a.feedforward_boost;
    t.tpa_rate = a.tpa_rate;
    t.tpa_breakpoint = a.tpa_breakpoint;
    let f = &s.filters;
    let ft = &mut t.filters;
    ft.gyro_lpf1_static_hz = f.gyro_lowpass_hz;
    ft.gyro_lpf1_type = bf_type(f.gyro_lowpass_type);
    ft.gyro_lpf1_dyn_min_hz = f.gyro_lowpass_dyn_min_hz;
    ft.gyro_lpf1_dyn_max_hz = f.gyro_lowpass_dyn_max_hz;
    ft.gyro_lpf2_static_hz = f.gyro_lowpass2_hz;
    ft.gyro_lpf2_type = bf_type(f.gyro_lowpass2_type);
    ft.gyro_notch1_hz = f.gyro_notch_hz;
    ft.gyro_notch1_cutoff = f.gyro_notch_cutoff;
    ft.gyro_notch2_hz = f.gyro_notch2_hz;
    ft.gyro_notch2_cutoff = f.gyro_notch2_cutoff;
    ft.dterm_lpf1_static_hz = f.dterm_lowpass_hz;
    ft.dterm_lpf1_type = bf_type(f.dterm_lowpass_type);
    ft.dterm_lpf1_dyn_min_hz = f.dterm_lowpass_dyn_min_hz;
    ft.dterm_lpf1_dyn_max_hz = f.dterm_lowpass_dyn_max_hz;
    ft.dterm_lpf2_static_hz = f.dterm_lowpass2_hz;
    ft.dterm_lpf2_type = bf_type(f.dterm_lowpass2_type);
    ft.dterm_notch_hz = f.dterm_notch_hz;
    ft.dterm_notch_cutoff = f.dterm_notch_cutoff;
    ft.dyn_notch_count = f.dyn_notch_count;
    ft.dyn_notch_q = f.dyn_notch_q;
    ft.dyn_notch_min_hz = f.dyn_notch_min_hz;
    ft.dyn_notch_max_hz = f.dyn_notch_max_hz;
    ft.rpm_filter_harmonics = f.gyro_rpm_notch_harmonics;
    ft.rpm_filter_min_hz = f.gyro_rpm_notch_min_hz as u16;
    if f.has_rpm_ext {
        ft.rpm_filter_q = f.gyro_rpm_notch_q;
        ft.rpm_filter_fade_range_hz = f.gyro_rpm_notch_fade_range_hz;
    }
    if let Some(sp) = &s.simplified {
        let st = &mut t.simplified;
        st.pids_mode = sp.pids_mode;
        st.master_multiplier = sp.master_multiplier;
        st.pi_gain = sp.pi_gain;
        st.i_gain = sp.i_gain;
        st.d_gain = sp.d_gain;
        st.d_max_gain = sp.dmax_gain;
        st.feedforward_gain = sp.feedforward_gain;
        st.pitch_pi_gain = sp.pitch_pi_gain;
        st.pitch_d_gain = sp.roll_pitch_ratio;
        st.gyro_filter = sp.gyro_filter != 0;
        st.gyro_filter_multiplier = sp.gyro_filter_multiplier;
        st.dterm_filter = sp.dterm_filter != 0;
        st.dterm_filter_multiplier = sp.dterm_filter_multiplier;
    } else {
        t.simplified.pids_mode = 0;
        t.simplified.gyro_filter = false;
        t.simplified.dterm_filter = false;
    }
    if let Some(bb) = &s.blackbox {
        t.raw
            .insert("blackbox_device".into(), bb.device.to_string());
        t.raw.insert(
            "blackbox_sample_rate".into(),
            format!("1/{}", bb.sample_divisor()),
        );
    }
    if let Some(ac) = &s.advanced_config {
        t.raw.insert("debug_mode".into(), ac.debug_mode.to_string());
        t.raw
            .insert("pid_process_denom".into(), ac.pid_process_denom.to_string());
    }
    t.api = (s.identity.api.major, s.identity.api.minor);
    t
}

/// Wait for a rebooting FC to come back and reopen the link.
pub fn reconnect(port: &str, timeout: Duration) -> Result<MspClient, MspError> {
    let deadline = std::time::Instant::now() + timeout;
    std::thread::sleep(Duration::from_millis(1500));
    loop {
        match MspClient::open(port) {
            Ok(c) => return Ok(c),
            Err(e) if std::time::Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

#[cfg(test)]
pub mod mock {
    //! An in-memory Betaflight that answers MSP v1 requests from its state.
    use super::*;
    use crate::codec::{encode_v2, Parser};
    use std::collections::VecDeque;

    pub struct FakeFc {
        pub api: ApiVersion,
        pub pids: Pids,
        pub adv: PidAdvanced,
        pub flt: FilterConfig,
        pub simp: SimplifiedTuning,
        pub bb: BlackboxConfig,
        pub ac: AdvancedConfig,
        pub armed: bool,
        pub eeprom_writes: u32,
        pub flash: Vec<u8>,
        parser: Parser,
        out: VecDeque<u8>,
        pub log: Vec<u16>,
    }

    impl FakeFc {
        pub fn new(api: ApiVersion) -> Self {
            let mut flt = FilterConfig {
                gyro_lowpass_hz: 250,
                gyro_lowpass_dyn_min_hz: 250,
                gyro_lowpass_dyn_max_hz: 500,
                gyro_lowpass2_hz: 500,
                dterm_lowpass_hz: 75,
                dterm_lowpass_dyn_min_hz: 75,
                dterm_lowpass_dyn_max_hz: 150,
                dterm_lowpass2_hz: 150,
                dyn_notch_count: 3,
                dyn_notch_q: 300,
                dyn_notch_min_hz: 100,
                dyn_notch_max_hz: 600,
                gyro_rpm_notch_harmonics: 3,
                gyro_rpm_notch_min_hz: 100,
                ..Default::default()
            };
            if api.at_least(1, 48) {
                flt.has_rpm_ext = true;
                flt.gyro_rpm_notch_q = 500;
                flt.gyro_rpm_notch_weights = [100, 100, 100];
            }
            Self {
                api,
                pids: Pids {
                    rows: vec![
                        [45, 80, 30],
                        [47, 84, 34],
                        [45, 80, 0],
                        [50, 50, 75],
                        [40, 0, 0],
                    ],
                },
                adv: PidAdvanced {
                    feedforward_roll: 120,
                    feedforward_pitch: 125,
                    feedforward_yaw: 120,
                    d_max_roll: 40,
                    d_max_pitch: 46,
                    d_max_gain: 37,
                    d_max_advance: 20,
                    anti_gravity_gain: 80,
                    iterm_relax: 1,
                    iterm_relax_cutoff: 15,
                    feedforward_smooth_factor: 65,
                    tpa_rate: 65,
                    tpa_breakpoint: 1350,
                    ..Default::default()
                },
                flt,
                simp: SimplifiedTuning {
                    pids_mode: 2,
                    master_multiplier: 100,
                    pi_gain: 100,
                    i_gain: 100,
                    d_gain: 100,
                    dmax_gain: 100,
                    feedforward_gain: 100,
                    pitch_pi_gain: 100,
                    roll_pitch_ratio: 100,
                    gyro_filter: 1,
                    gyro_filter_multiplier: 100,
                    dterm_filter: 1,
                    dterm_filter_multiplier: 100,
                    ..Default::default()
                },
                bb: BlackboxConfig {
                    supported: true,
                    device: 1,
                    rate_num: 1,
                    rate_denom: 1,
                    p_denom: 32,
                    sample_rate: 2,
                    disabled_mask: Some(0),
                },
                ac: AdvancedConfig {
                    gyro_sync_denom: 1,
                    pid_process_denom: 1,
                    debug_mode: 6,
                    debug_mode_count: 70,
                    ..Default::default()
                },
                armed: false,
                eeprom_writes: 0,
                flash: (0..10_000u32).map(|i| (i % 251) as u8).collect(),
                parser: Parser::default(),
                out: VecDeque::new(),
                log: Vec::new(),
            }
        }

        fn reply(&mut self, cmd: u16, payload: &[u8]) {
            let mut f = encode_v2(cmd, payload);
            f[2] = b'>';
            self.out.extend(f);
        }

        fn handle(&mut self, cmd: u16, p: &[u8]) {
            self.log.push(cmd);
            let api = self.api;
            match cmd {
                MSP_API_VERSION => self.reply(cmd, &[0, api.major, api.minor]),
                MSP_FC_VARIANT => self.reply(cmd, b"BTFL"),
                MSP_FC_VERSION => self.reply(cmd, &[4, 5, 1]),
                MSP_BOARD_INFO => self.reply(
                    cmd,
                    &[
                        b'S', b'P', b'B', b'E', 0, 0, 0, 0, 4, b'T', b'E', b'S', b'T', 0,
                    ],
                ),
                MSP_STATUS_EX => {
                    let mut w = crate::codec::Writer::new();
                    w.u16(125)
                        .u16(0)
                        .u16(0)
                        .u32(if self.armed { 1 } else { 0 })
                        .u8(0)
                        .u16(10)
                        .u8(4)
                        .u8(0)
                        .u8(0)
                        .u8(0)
                        .u32(0)
                        .u8(0);
                    self.reply(cmd, &w.0);
                }
                MSP_PID => {
                    let b = self.pids.encode();
                    self.reply(cmd, &b)
                }
                MSP_SET_PID => {
                    self.pids = Pids::parse(p).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_PID_ADVANCED => {
                    let b = self.adv.encode(api);
                    self.reply(cmd, &b)
                }
                MSP_SET_PID_ADVANCED => {
                    self.adv = PidAdvanced::parse(p, api).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_FILTER_CONFIG => {
                    let b = self.flt.encode(api);
                    self.reply(cmd, &b)
                }
                MSP_SET_FILTER_CONFIG => {
                    self.flt = FilterConfig::parse(p, api).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_SIMPLIFIED_TUNING => {
                    let b = self.simp.encode();
                    self.reply(cmd, &b)
                }
                MSP_SET_SIMPLIFIED_TUNING => {
                    self.simp = SimplifiedTuning::parse(p).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_BLACKBOX_CONFIG => {
                    let mut b = vec![1u8];
                    b.extend(self.bb.encode(api));
                    self.reply(cmd, &b)
                }
                MSP_SET_BLACKBOX_CONFIG => {
                    self.bb = BlackboxConfig::parse(&[[1u8].as_slice(), p].concat(), api).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_ADVANCED_CONFIG => {
                    let mut b = self.ac.encode();
                    b.push(self.ac.debug_mode_count);
                    self.reply(cmd, &b)
                }
                MSP_SET_ADVANCED_CONFIG => {
                    self.ac =
                        AdvancedConfig::parse(&[p, &[self.ac.debug_mode_count]].concat()).unwrap();
                    self.reply(cmd, &[])
                }
                MSP_EEPROM_WRITE => {
                    self.eeprom_writes += 1;
                    self.reply(cmd, &[])
                }
                MSP_DATAFLASH_SUMMARY => {
                    let mut w = crate::codec::Writer::new();
                    w.u8(3).u32(1).u32(16 << 20).u32(self.flash.len() as u32);
                    self.reply(cmd, &w.0);
                }
                MSP_DATAFLASH_READ => {
                    let mut r = crate::codec::Reader::new(p);
                    let addr = r.u32().unwrap() as usize;
                    let len = r.u16().unwrap() as usize;
                    let end = (addr + len).min(self.flash.len());
                    let chunk = self.flash[addr.min(end)..end].to_vec();
                    let mut w = crate::codec::Writer::new();
                    w.u32(addr as u32).u16(chunk.len() as u16).u8(0);
                    w.0.extend_from_slice(&chunk);
                    self.reply(cmd, &w.0);
                }
                _ => {
                    // error reply: v1 '!' direction
                    let mut f = encode_v2(cmd, &[]);
                    f[2] = b'!';
                    self.out.extend(f);
                }
            }
        }
    }

    impl Transport for FakeFc {
        fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
            self.parser.push(data);
            while let Ok(Some(f)) = self.parser.next() {
                let p = f.payload.clone();
                self.handle(f.cmd, &p);
            }
            Ok(())
        }
        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.out.len());
            for b in buf.iter_mut().take(n) {
                *b = self.out.pop_front().unwrap();
            }
            Ok(n)
        }
        fn name(&self) -> String {
            "fake".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::FakeFc;
    use super::*;
    use uuid::Uuid;

    fn rec(name: &str, old: ParamValue, new: ParamValue) -> Recommendation {
        Recommendation {
            id: Uuid::new_v4(),
            param: ParamRef::Bf(name.into()),
            old,
            new,
            reason: "t".into(),
            evidence: vec![],
            confidence: Confidence::High,
            requires_reboot: false,
            accepted: true,
        }
    }

    #[test]
    fn handshake_and_read_tune() {
        let fc = FakeFc::new(ApiVersion::new(1, 46));
        let mut c = MspClient::with_transport(Box::new(fc), "fake").unwrap();
        assert_eq!(c.identity.version, "4.5.1");
        assert_eq!(c.identity.board_id, "SPBE");
        let t = c.read_tune().unwrap();
        // API 1.46 is legacy naming: MSP row D (34) is D Max, the dMax field (46) is Derivative.
        assert_eq!(t.pids[1].d, 46);
        assert_eq!(t.pids[1].d_max, 34);
        assert_eq!(t.filters.dyn_notch_count, 3);
        assert_eq!(t.simplified.pids_mode, 2);
        assert_eq!(t.get_raw("blackbox_sample_rate"), Some("1/4"));
        let r = c.blackbox_rate_hz().unwrap().unwrap();
        assert!((r - 2000.0).abs() < 1.0, "{r}");
    }

    #[test]
    fn apply_writes_and_verifies() {
        for api in [
            ApiVersion::new(1, 44),
            ApiVersion::new(1, 46),
            ApiVersion::new(1, 48),
        ] {
            let fc = FakeFc::new(api);
            let mut c = MspClient::with_transport(Box::new(fc), "fake").unwrap();
            let recs = vec![
                rec(
                    "simplified_pids_mode",
                    ParamValue::Enum(2),
                    ParamValue::Enum(0),
                ),
                rec(
                    if api.at_least(1, 47) {
                        "d_pitch"
                    } else {
                        "d_min_pitch"
                    },
                    ParamValue::U8(34),
                    ParamValue::U8(38),
                ),
                rec(
                    if api.at_least(1, 47) {
                        "d_max_pitch"
                    } else {
                        "d_pitch"
                    },
                    ParamValue::U8(46),
                    ParamValue::U8(50),
                ),
                rec("f_roll", ParamValue::U16(120), ParamValue::U16(135)),
                rec("dyn_notch_count", ParamValue::U8(3), ParamValue::U8(2)),
                rec(
                    "gyro_lpf1_dyn_max_hz",
                    ParamValue::U16(500),
                    ParamValue::U16(400),
                ),
            ];
            let r = c.apply(&recs).unwrap();
            assert!(r.verified, "{api:?}: {:?}", r.outcomes);
            assert_eq!(r.outcomes.len(), 6);
            assert!(!r.rebooted);
            let t = c.read_tune().unwrap();
            assert_eq!(t.pids[1].d, 38, "{api:?}");
            assert_eq!(t.pids[1].d_max, 50, "{api:?}");
            assert_eq!(t.pids[0].ff, 135);
            assert_eq!(t.filters.dyn_notch_count, 2);
            assert_eq!(t.simplified.pids_mode, 0);
            // untouched fields survive the read-modify-write
            if api.at_least(1, 47) {
                assert_eq!(t.pids[0].d, 30);
            } else {
                assert_eq!(t.pids[0].d_max, 30);
            }
            assert_eq!(t.filters.gyro_lpf2_static_hz, 500);
            assert_eq!(t.anti_gravity_gain, 80);
        }
    }

    #[test]
    fn apply_refuses_when_armed() {
        let mut fc = FakeFc::new(ApiVersion::new(1, 46));
        fc.armed = true;
        let mut c = MspClient::with_transport(Box::new(fc), "fake").unwrap();
        let e = c
            .apply(&[rec("d_roll", ParamValue::U8(30), ParamValue::U8(31))])
            .unwrap_err();
        assert!(matches!(e, MspError::Refused(_)));
    }

    #[test]
    fn dataflash_download_roundtrip() {
        let fc = FakeFc::new(ApiVersion::new(1, 46));
        let expected = fc.flash.clone();
        let mut c = MspClient::with_transport(Box::new(fc), "fake").unwrap();
        let mut last = 0;
        let data = c.dataflash_download(|a, _| last = a).unwrap();
        assert_eq!(data, expected);
        assert_eq!(last as usize, expected.len());
    }
}
