//! MSP message layouts (transcribed from betaflight-configurator MSPHelper.js).
//! Every struct is read with the full field list and written back verbatim so
//! a partial edit never resets other fields ("read-modify-write").

use crate::codec::{MspError, Reader, Writer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct ApiVersion {
    pub major: u8,
    pub minor: u8,
}

impl ApiVersion {
    pub const fn new(major: u8, minor: u8) -> Self {
        Self { major, minor }
    }
    pub fn at_least(self, major: u8, minor: u8) -> bool {
        self >= ApiVersion::new(major, minor)
    }
}

fn short(what: &'static str, got: usize) -> MspError {
    MspError::Short { what, got }
}

macro_rules! rd {
    ($r:expr, $what:expr, $m:ident) => {
        $r.$m().ok_or_else(|| short($what, $r.data.len()))?
    };
}

// ---------------------------------------------------------------------------
// Identity / status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Identity {
    pub msp_protocol: u8,
    pub api: ApiVersion,
    pub variant: String,
    pub version: String,
    pub board_id: String,
    pub target_name: String,
    pub board_name: String,
}

pub fn parse_api_version(p: &[u8]) -> Result<(u8, ApiVersion), MspError> {
    let mut r = Reader::new(p);
    let proto = rd!(r, "MSP_API_VERSION", u8);
    let major = rd!(r, "MSP_API_VERSION", u8);
    let minor = rd!(r, "MSP_API_VERSION", u8);
    Ok((proto, ApiVersion::new(major, minor)))
}

pub fn parse_fc_variant(p: &[u8]) -> Result<String, MspError> {
    if p.len() < 4 {
        return Err(short("MSP_FC_VARIANT", p.len()));
    }
    Ok(String::from_utf8_lossy(&p[..4]).into_owned())
}

pub fn parse_fc_version(p: &[u8]) -> Result<String, MspError> {
    let mut r = Reader::new(p);
    let major = rd!(r, "MSP_FC_VERSION", u8);
    if major < 10 {
        let minor = rd!(r, "MSP_FC_VERSION", u8);
        let patch = rd!(r, "MSP_FC_VERSION", u8);
        Ok(format!("{major}.{minor}.{patch}"))
    } else {
        // calendar versions: two discarded bytes then a length-prefixed string
        let _ = rd!(r, "MSP_FC_VERSION", u16);
        Ok(r.text().unwrap_or_else(|| format!("{major}")))
    }
}

pub fn parse_board_info(p: &[u8]) -> Result<(String, String, String), MspError> {
    let mut r = Reader::new(p);
    let id = String::from_utf8_lossy(r.bytes(4).ok_or_else(|| short("MSP_BOARD_INFO", p.len()))?)
        .into_owned();
    let _board_version = rd!(r, "MSP_BOARD_INFO", u16);
    let _board_type = rd!(r, "MSP_BOARD_INFO", u8);
    let _caps = rd!(r, "MSP_BOARD_INFO", u8);
    let target = r.text().unwrap_or_default();
    let name = r.text().unwrap_or_default();
    Ok((id, target, name))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusEx {
    pub cycle_time_us: u16,
    pub i2c_errors: u16,
    pub active_sensors: u16,
    /// Box flags; bit 0 = ARM.
    pub mode: u32,
    pub profile: u8,
    pub cpu_load: u16,
    pub num_profiles: u8,
    pub rate_profile: u8,
    pub arming_disable_count: u8,
    pub arming_disable_flags: u32,
    pub config_state: u8,
}

impl StatusEx {
    pub fn armed(&self) -> bool {
        self.mode & 1 != 0
    }
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        let mut r = Reader::new(p);
        let mut s = StatusEx {
            cycle_time_us: rd!(r, "MSP_STATUS_EX", u16),
            i2c_errors: rd!(r, "MSP_STATUS_EX", u16),
            active_sensors: rd!(r, "MSP_STATUS_EX", u16),
            mode: rd!(r, "MSP_STATUS_EX", u32),
            profile: rd!(r, "MSP_STATUS_EX", u8),
            cpu_load: rd!(r, "MSP_STATUS_EX", u16),
            num_profiles: rd!(r, "MSP_STATUS_EX", u8),
            rate_profile: rd!(r, "MSP_STATUS_EX", u8),
            ..Default::default()
        };
        let extra = rd!(r, "MSP_STATUS_EX", u8) as usize; // extra flight-mode flag bytes
        r.bytes(extra)
            .ok_or_else(|| short("MSP_STATUS_EX", p.len()))?;
        s.arming_disable_count = rd!(r, "MSP_STATUS_EX", u8);
        s.arming_disable_flags = rd!(r, "MSP_STATUS_EX", u32);
        s.config_state = r.u8().unwrap_or(0);
        Ok(s)
    }
}

// ---------------------------------------------------------------------------
// PIDs
// ---------------------------------------------------------------------------

/// `MSP_PID`: rows ROLL, PITCH, YAW, LEVEL, MAG (5 × [P, I, D]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pids {
    pub rows: Vec<[u8; 3]>,
}

impl Pids {
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        if p.len() < 9 {
            return Err(short("MSP_PID", p.len()));
        }
        Ok(Self {
            rows: p.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        self.rows.iter().flatten().copied().collect()
    }
}

/// `MSP_PID_ADVANCED` (API-gated). Field names follow MSPHelper.js.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PidAdvanced {
    pub roll_pitch_iterm_ignore_rate: u16,
    pub yaw_iterm_ignore_rate: u16,
    pub yaw_p_limit: u16,
    pub delta_method: u8,
    pub vbat_pid_compensation: u8,
    pub feedforward_transition: u8,
    pub dterm_setpoint_weight8: u8,
    pub tolerance_band: u8,
    pub tolerance_band_reduction: u8,
    pub iterm_throttle_gain: u8,
    pub pid_max_velocity: u16,
    pub pid_max_velocity_yaw: u16,
    pub level_angle_limit: u8,
    pub level_sensitivity: u8,
    pub iterm_throttle_threshold: u16,
    /// `anti_gravity_gain` (≥1.45) or `itermAcceleratorGain` (older).
    pub anti_gravity_gain: u16,
    pub dterm_setpoint_weight: u16,
    pub iterm_rotation: u8,
    pub smart_feedforward: u8,
    pub iterm_relax: u8,
    pub iterm_relax_type: u8,
    pub absolute_control_gain: u8,
    pub throttle_boost: u8,
    pub acro_trainer_angle_limit: u8,
    pub feedforward_roll: u16,
    pub feedforward_pitch: u16,
    pub feedforward_yaw: u16,
    pub anti_gravity_mode: u8,
    pub d_max_roll: u8,
    pub d_max_pitch: u8,
    pub d_max_yaw: u8,
    pub d_max_gain: u8,
    pub d_max_advance: u8,
    pub use_integrated_yaw: u8,
    pub integrated_yaw_relax: u8,
    // ≥ 1.42
    pub iterm_relax_cutoff: u8,
    // ≥ 1.43
    pub motor_output_limit: u8,
    pub auto_profile_cell_count: i8,
    pub idle_min_rpm: u8,
    // ≥ 1.44
    pub feedforward_averaging: u8,
    pub feedforward_smooth_factor: u8,
    pub feedforward_boost: u8,
    pub feedforward_max_rate_limit: u8,
    pub feedforward_jitter_factor: u8,
    pub vbat_sag_compensation: u8,
    pub thrust_linearization: u8,
    // ≥ 1.45
    pub tpa_mode: u8,
    pub tpa_rate: u8,
    pub tpa_breakpoint: u16,
    /// Bytes after the last field we know, kept for round-tripping newer firmware.
    pub tail: Vec<u8>,
}

impl PidAdvanced {
    pub fn parse(p: &[u8], api: ApiVersion) -> Result<Self, MspError> {
        let w = "MSP_PID_ADVANCED";
        let mut r = Reader::new(p);
        let mut a = PidAdvanced {
            roll_pitch_iterm_ignore_rate: rd!(r, w, u16),
            yaw_iterm_ignore_rate: rd!(r, w, u16),
            yaw_p_limit: rd!(r, w, u16),
            delta_method: rd!(r, w, u8),
            vbat_pid_compensation: rd!(r, w, u8),
            feedforward_transition: rd!(r, w, u8),
            dterm_setpoint_weight8: rd!(r, w, u8),
            tolerance_band: rd!(r, w, u8),
            tolerance_band_reduction: rd!(r, w, u8),
            iterm_throttle_gain: rd!(r, w, u8),
            pid_max_velocity: rd!(r, w, u16),
            pid_max_velocity_yaw: rd!(r, w, u16),
            level_angle_limit: rd!(r, w, u8),
            level_sensitivity: rd!(r, w, u8),
            iterm_throttle_threshold: rd!(r, w, u16),
            anti_gravity_gain: rd!(r, w, u16),
            dterm_setpoint_weight: rd!(r, w, u16),
            iterm_rotation: rd!(r, w, u8),
            smart_feedforward: rd!(r, w, u8),
            iterm_relax: rd!(r, w, u8),
            iterm_relax_type: rd!(r, w, u8),
            absolute_control_gain: rd!(r, w, u8),
            throttle_boost: rd!(r, w, u8),
            acro_trainer_angle_limit: rd!(r, w, u8),
            feedforward_roll: rd!(r, w, u16),
            feedforward_pitch: rd!(r, w, u16),
            feedforward_yaw: rd!(r, w, u16),
            anti_gravity_mode: rd!(r, w, u8),
            d_max_roll: rd!(r, w, u8),
            d_max_pitch: rd!(r, w, u8),
            d_max_yaw: rd!(r, w, u8),
            d_max_gain: rd!(r, w, u8),
            d_max_advance: rd!(r, w, u8),
            use_integrated_yaw: rd!(r, w, u8),
            integrated_yaw_relax: rd!(r, w, u8),
            ..Default::default()
        };
        if api.at_least(1, 42) {
            a.iterm_relax_cutoff = rd!(r, w, u8);
        }
        if api.at_least(1, 43) {
            a.motor_output_limit = rd!(r, w, u8);
            a.auto_profile_cell_count = rd!(r, w, i8);
            a.idle_min_rpm = rd!(r, w, u8);
        }
        if api.at_least(1, 44) {
            a.feedforward_averaging = rd!(r, w, u8);
            a.feedforward_smooth_factor = rd!(r, w, u8);
            a.feedforward_boost = rd!(r, w, u8);
            a.feedforward_max_rate_limit = rd!(r, w, u8);
            a.feedforward_jitter_factor = rd!(r, w, u8);
            a.vbat_sag_compensation = rd!(r, w, u8);
            a.thrust_linearization = rd!(r, w, u8);
        }
        if api.at_least(1, 45) {
            a.tpa_mode = rd!(r, w, u8);
            a.tpa_rate = rd!(r, w, u8);
            a.tpa_breakpoint = rd!(r, w, u16);
        }
        a.tail = p[r.pos..].to_vec();
        Ok(a)
    }

    pub fn encode(&self, api: ApiVersion) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(self.roll_pitch_iterm_ignore_rate)
            .u16(self.yaw_iterm_ignore_rate)
            .u16(self.yaw_p_limit)
            .u8(self.delta_method)
            .u8(self.vbat_pid_compensation)
            .u8(self.feedforward_transition)
            .u8(self.dterm_setpoint_weight8.min(254))
            .u8(self.tolerance_band)
            .u8(self.tolerance_band_reduction)
            .u8(self.iterm_throttle_gain)
            .u16(self.pid_max_velocity)
            .u16(self.pid_max_velocity_yaw)
            .u8(self.level_angle_limit)
            .u8(self.level_sensitivity)
            .u16(self.iterm_throttle_threshold)
            .u16(self.anti_gravity_gain)
            .u16(self.dterm_setpoint_weight)
            .u8(self.iterm_rotation)
            .u8(self.smart_feedforward)
            .u8(self.iterm_relax)
            .u8(self.iterm_relax_type)
            .u8(if api.at_least(1, 48) {
                0
            } else {
                self.absolute_control_gain
            })
            .u8(self.throttle_boost)
            .u8(self.acro_trainer_angle_limit)
            .u16(self.feedforward_roll)
            .u16(self.feedforward_pitch)
            .u16(self.feedforward_yaw)
            .u8(self.anti_gravity_mode)
            .u8(self.d_max_roll)
            .u8(self.d_max_pitch)
            .u8(self.d_max_yaw)
            .u8(self.d_max_gain)
            .u8(self.d_max_advance)
            .u8(self.use_integrated_yaw)
            .u8(self.integrated_yaw_relax);
        if api.at_least(1, 42) {
            w.u8(self.iterm_relax_cutoff);
        }
        if api.at_least(1, 43) {
            w.u8(self.motor_output_limit)
                .i8(self.auto_profile_cell_count)
                .u8(self.idle_min_rpm);
        }
        if api.at_least(1, 44) {
            w.u8(self.feedforward_averaging)
                .u8(self.feedforward_smooth_factor)
                .u8(self.feedforward_boost)
                .u8(self.feedforward_max_rate_limit)
                .u8(self.feedforward_jitter_factor)
                .u8(self.vbat_sag_compensation)
                .u8(self.thrust_linearization);
        }
        if api.at_least(1, 45) {
            w.u8(self.tpa_mode)
                .u8(self.tpa_rate)
                .u16(self.tpa_breakpoint);
        }
        w.0.extend_from_slice(&self.tail);
        w.0
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    pub gyro_lowpass_hz: u16,
    pub dterm_lowpass_hz: u16,
    pub yaw_lowpass_hz: u16,
    pub gyro_notch_hz: u16,
    pub gyro_notch_cutoff: u16,
    pub dterm_notch_hz: u16,
    pub dterm_notch_cutoff: u16,
    pub gyro_notch2_hz: u16,
    pub gyro_notch2_cutoff: u16,
    pub dterm_lowpass_type: u8,
    pub gyro_hardware_lpf: u8,
    pub gyro_lowpass2_hz: u16,
    pub gyro_lowpass_type: u8,
    pub gyro_lowpass2_type: u8,
    pub dterm_lowpass2_hz: u16,
    pub dterm_lowpass2_type: u8,
    pub gyro_lowpass_dyn_min_hz: u16,
    pub gyro_lowpass_dyn_max_hz: u16,
    pub dterm_lowpass_dyn_min_hz: u16,
    pub dterm_lowpass_dyn_max_hz: u16,
    // ≥ 1.42
    pub dyn_notch_range: u8,
    pub dyn_notch_width_percent: u8,
    pub dyn_notch_q: u16,
    pub dyn_notch_min_hz: u16,
    pub gyro_rpm_notch_harmonics: u8,
    pub gyro_rpm_notch_min_hz: u8,
    // ≥ 1.43
    pub dyn_notch_max_hz: u16,
    // ≥ 1.44
    pub dyn_lpf_curve_expo: u8,
    pub dyn_notch_count: u8,
    // ≥ 1.48
    pub has_rpm_ext: bool,
    pub gyro_rpm_notch_fade_range_hz: u16,
    pub gyro_rpm_notch_q: u16,
    pub gyro_rpm_notch_weights: [u8; 3],
    pub tail: Vec<u8>,
}

impl FilterConfig {
    pub fn parse(p: &[u8], api: ApiVersion) -> Result<Self, MspError> {
        let w = "MSP_FILTER_CONFIG";
        let mut r = Reader::new(p);
        let mut f = FilterConfig::default();
        let _legacy_gyro8 = rd!(r, w, u8);
        f.dterm_lowpass_hz = rd!(r, w, u16);
        f.yaw_lowpass_hz = rd!(r, w, u16);
        f.gyro_notch_hz = rd!(r, w, u16);
        f.gyro_notch_cutoff = rd!(r, w, u16);
        f.dterm_notch_hz = rd!(r, w, u16);
        f.dterm_notch_cutoff = rd!(r, w, u16);
        f.gyro_notch2_hz = rd!(r, w, u16);
        f.gyro_notch2_cutoff = rd!(r, w, u16);
        f.dterm_lowpass_type = rd!(r, w, u8);
        f.gyro_hardware_lpf = rd!(r, w, u8);
        let _gyro32k = rd!(r, w, u8);
        f.gyro_lowpass_hz = rd!(r, w, u16);
        f.gyro_lowpass2_hz = rd!(r, w, u16);
        f.gyro_lowpass_type = rd!(r, w, u8);
        f.gyro_lowpass2_type = rd!(r, w, u8);
        f.dterm_lowpass2_hz = rd!(r, w, u16);
        f.dterm_lowpass2_type = rd!(r, w, u8);
        f.gyro_lowpass_dyn_min_hz = rd!(r, w, u16);
        f.gyro_lowpass_dyn_max_hz = rd!(r, w, u16);
        f.dterm_lowpass_dyn_min_hz = rd!(r, w, u16);
        f.dterm_lowpass_dyn_max_hz = rd!(r, w, u16);
        if api.at_least(1, 42) {
            f.dyn_notch_range = rd!(r, w, u8);
            f.dyn_notch_width_percent = rd!(r, w, u8);
            f.dyn_notch_q = rd!(r, w, u16);
            f.dyn_notch_min_hz = rd!(r, w, u16);
            f.gyro_rpm_notch_harmonics = rd!(r, w, u8);
            f.gyro_rpm_notch_min_hz = rd!(r, w, u8);
        }
        if api.at_least(1, 43) {
            f.dyn_notch_max_hz = rd!(r, w, u16);
        }
        if api.at_least(1, 44) {
            f.dyn_lpf_curve_expo = rd!(r, w, u8);
            f.dyn_notch_count = rd!(r, w, u8);
        }
        if api.at_least(1, 48) && r.remaining() >= 7 {
            f.has_rpm_ext = true;
            f.gyro_rpm_notch_fade_range_hz = rd!(r, w, u16);
            f.gyro_rpm_notch_q = rd!(r, w, u16);
            for k in 0..3 {
                f.gyro_rpm_notch_weights[k] = rd!(r, w, u8);
            }
        }
        f.tail = p[r.pos..].to_vec();
        Ok(f)
    }

    pub fn encode(&self, api: ApiVersion) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.gyro_lowpass_hz.min(255) as u8)
            .u16(self.dterm_lowpass_hz)
            .u16(self.yaw_lowpass_hz)
            .u16(self.gyro_notch_hz)
            .u16(self.gyro_notch_cutoff)
            .u16(self.dterm_notch_hz)
            .u16(self.dterm_notch_cutoff)
            .u16(self.gyro_notch2_hz)
            .u16(self.gyro_notch2_cutoff)
            .u8(self.dterm_lowpass_type)
            .u8(self.gyro_hardware_lpf)
            .u8(0)
            .u16(self.gyro_lowpass_hz)
            .u16(self.gyro_lowpass2_hz)
            .u8(self.gyro_lowpass_type)
            .u8(self.gyro_lowpass2_type)
            .u16(self.dterm_lowpass2_hz)
            .u8(self.dterm_lowpass2_type)
            .u16(self.gyro_lowpass_dyn_min_hz)
            .u16(self.gyro_lowpass_dyn_max_hz)
            .u16(self.dterm_lowpass_dyn_min_hz)
            .u16(self.dterm_lowpass_dyn_max_hz);
        if api.at_least(1, 42) {
            w.u8(self.dyn_notch_range)
                .u8(self.dyn_notch_width_percent)
                .u16(self.dyn_notch_q)
                .u16(self.dyn_notch_min_hz)
                .u8(self.gyro_rpm_notch_harmonics)
                .u8(self.gyro_rpm_notch_min_hz);
        }
        if api.at_least(1, 43) {
            w.u16(self.dyn_notch_max_hz);
        }
        if api.at_least(1, 44) {
            w.u8(self.dyn_lpf_curve_expo).u8(self.dyn_notch_count);
        }
        if api.at_least(1, 48) && self.has_rpm_ext {
            w.u16(self.gyro_rpm_notch_fade_range_hz)
                .u16(self.gyro_rpm_notch_q);
            for k in 0..3 {
                w.u8(self.gyro_rpm_notch_weights[k]);
            }
        }
        w.0.extend_from_slice(&self.tail);
        w.0
    }
}

// ---------------------------------------------------------------------------
// Simplified tuning sliders (≥ 1.44)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SimplifiedTuning {
    pub pids_mode: u8,
    pub master_multiplier: u8,
    pub roll_pitch_ratio: u8,
    pub i_gain: u8,
    pub d_gain: u8,
    pub pi_gain: u8,
    pub dmax_gain: u8,
    pub feedforward_gain: u8,
    pub pitch_pi_gain: u8,
    pub dterm_filter: u8,
    pub dterm_filter_multiplier: u8,
    pub dterm_lowpass_hz: u16,
    pub dterm_lowpass2_hz: u16,
    pub dterm_lowpass_dyn_min_hz: u16,
    pub dterm_lowpass_dyn_max_hz: u16,
    pub gyro_filter: u8,
    pub gyro_filter_multiplier: u8,
    pub gyro_lowpass_hz: u16,
    pub gyro_lowpass2_hz: u16,
    pub gyro_lowpass_dyn_min_hz: u16,
    pub gyro_lowpass_dyn_max_hz: u16,
}

impl SimplifiedTuning {
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        let w = "MSP_SIMPLIFIED_TUNING";
        let mut r = Reader::new(p);
        let mut s = SimplifiedTuning {
            pids_mode: rd!(r, w, u8),
            master_multiplier: rd!(r, w, u8),
            roll_pitch_ratio: rd!(r, w, u8),
            i_gain: rd!(r, w, u8),
            d_gain: rd!(r, w, u8),
            pi_gain: rd!(r, w, u8),
            dmax_gain: rd!(r, w, u8),
            feedforward_gain: rd!(r, w, u8),
            pitch_pi_gain: rd!(r, w, u8),
            ..Default::default()
        };
        rd!(r, w, u32);
        rd!(r, w, u32);
        s.dterm_filter = rd!(r, w, u8);
        s.dterm_filter_multiplier = rd!(r, w, u8);
        s.dterm_lowpass_hz = rd!(r, w, u16);
        s.dterm_lowpass2_hz = rd!(r, w, u16);
        s.dterm_lowpass_dyn_min_hz = rd!(r, w, u16);
        s.dterm_lowpass_dyn_max_hz = rd!(r, w, u16);
        rd!(r, w, u32);
        rd!(r, w, u32);
        s.gyro_filter = rd!(r, w, u8);
        s.gyro_filter_multiplier = rd!(r, w, u8);
        s.gyro_lowpass_hz = rd!(r, w, u16);
        s.gyro_lowpass2_hz = rd!(r, w, u16);
        s.gyro_lowpass_dyn_min_hz = rd!(r, w, u16);
        s.gyro_lowpass_dyn_max_hz = rd!(r, w, u16);
        Ok(s)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.pids_mode)
            .u8(self.master_multiplier)
            .u8(self.roll_pitch_ratio)
            .u8(self.i_gain)
            .u8(self.d_gain)
            .u8(self.pi_gain)
            .u8(self.dmax_gain)
            .u8(self.feedforward_gain)
            .u8(self.pitch_pi_gain)
            .u32(0)
            .u32(0)
            .u8(self.dterm_filter)
            .u8(self.dterm_filter_multiplier)
            .u16(self.dterm_lowpass_hz)
            .u16(self.dterm_lowpass2_hz)
            .u16(self.dterm_lowpass_dyn_min_hz)
            .u16(self.dterm_lowpass_dyn_max_hz)
            .u32(0)
            .u32(0)
            .u8(self.gyro_filter)
            .u8(self.gyro_filter_multiplier)
            .u16(self.gyro_lowpass_hz)
            .u16(self.gyro_lowpass2_hz)
            .u16(self.gyro_lowpass_dyn_min_hz)
            .u16(self.gyro_lowpass_dyn_max_hz)
            .u32(0)
            .u32(0);
        w.0
    }
}

// ---------------------------------------------------------------------------
// Blackbox / storage / advanced config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BlackboxConfig {
    pub supported: bool,
    /// 0 NONE, 1 FLASH, 2 SDCARD, 3 SERIAL
    pub device: u8,
    pub rate_num: u8,
    pub rate_denom: u8,
    pub p_denom: u16,
    /// ≥1.44: 0 = 1/1, 1 = 1/2, 2 = 1/4, 3 = 1/8, 4 = 1/16
    pub sample_rate: u8,
    pub disabled_mask: Option<u32>,
}

impl BlackboxConfig {
    pub fn parse(p: &[u8], api: ApiVersion) -> Result<Self, MspError> {
        let w = "MSP_BLACKBOX_CONFIG";
        let mut r = Reader::new(p);
        let mut b = BlackboxConfig {
            supported: rd!(r, w, u8) & 1 != 0,
            device: rd!(r, w, u8),
            rate_num: rd!(r, w, u8),
            rate_denom: rd!(r, w, u8),
            p_denom: rd!(r, w, u16),
            ..Default::default()
        };
        if api.at_least(1, 44) {
            b.sample_rate = r.u8().unwrap_or(0);
        }
        if api.at_least(1, 45) {
            b.disabled_mask = r.u32();
        }
        Ok(b)
    }
    pub fn encode(&self, api: ApiVersion) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.device)
            .u8(self.rate_num)
            .u8(self.rate_denom)
            .u16(self.p_denom);
        if api.at_least(1, 44) {
            w.u8(self.sample_rate);
        }
        if api.at_least(1, 45) {
            w.u32(self.disabled_mask.unwrap_or(0));
        }
        w.0
    }
    /// Sample-rate divisor: 1, 2, 4, 8, 16.
    pub fn sample_divisor(&self) -> u32 {
        1u32 << self.sample_rate.min(4)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataflashSummary {
    pub ready: bool,
    pub supported: bool,
    pub sectors: u32,
    pub total_size: u32,
    pub used_size: u32,
}

impl DataflashSummary {
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        if p.len() < 13 {
            return Ok(Self::default());
        }
        let mut r = Reader::new(p);
        let flags = r.u8().unwrap();
        Ok(Self {
            ready: flags & 1 != 0,
            supported: flags & 2 != 0,
            sectors: r.u32().unwrap(),
            total_size: r.u32().unwrap(),
            used_size: r.u32().unwrap(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SdcardSummary {
    pub supported: bool,
    pub state: u8,
    pub last_error: u8,
    pub free_kb: u32,
    pub total_kb: u32,
}

impl SdcardSummary {
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        let w = "MSP_SDCARD_SUMMARY";
        let mut r = Reader::new(p);
        Ok(Self {
            supported: rd!(r, w, u8) & 1 != 0,
            state: rd!(r, w, u8),
            last_error: rd!(r, w, u8),
            free_kb: rd!(r, w, u32),
            total_kb: rd!(r, w, u32),
        })
    }
}

/// Reply to `MSP_DATAFLASH_READ` (non-legacy form).
pub struct DataflashChunk {
    pub address: u32,
    pub data: Vec<u8>,
}

pub fn encode_dataflash_read(address: u32, len: u16) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(address).u16(len).u8(0); // no compression
    w.0
}

pub fn parse_dataflash_read(p: &[u8]) -> Result<DataflashChunk, MspError> {
    let mut r = Reader::new(p);
    let address = rd!(r, "MSP_DATAFLASH_READ", u32);
    let len = rd!(r, "MSP_DATAFLASH_READ", u16) as usize;
    let compression = rd!(r, "MSP_DATAFLASH_READ", u8);
    if compression != 0 {
        return Err(MspError::Protocol(format!(
            "unexpected dataflash compression {compression}"
        )));
    }
    let data = r
        .bytes(len)
        .ok_or_else(|| short("MSP_DATAFLASH_READ", p.len()))?
        .to_vec();
    Ok(DataflashChunk { address, data })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AdvancedConfig {
    pub gyro_sync_denom: u8,
    pub pid_process_denom: u8,
    pub use_unsynced_pwm: u8,
    pub fast_pwm_protocol: u8,
    pub motor_pwm_rate: u16,
    pub motor_idle: u16,
    pub gyro_use_32khz: u8,
    pub motor_pwm_inversion: u8,
    pub gyro_to_use: u8,
    pub gyro_high_fsr: u8,
    pub gyro_movement_calib_threshold: u8,
    pub gyro_calib_duration: u16,
    pub gyro_offset_yaw: u16,
    pub gyro_check_overflow: u8,
    pub debug_mode: u8,
    pub debug_mode_count: u8,
    pub tail: Vec<u8>,
}

impl AdvancedConfig {
    pub fn parse(p: &[u8]) -> Result<Self, MspError> {
        let w = "MSP_ADVANCED_CONFIG";
        let mut r = Reader::new(p);
        let mut a = AdvancedConfig {
            gyro_sync_denom: rd!(r, w, u8),
            pid_process_denom: rd!(r, w, u8),
            use_unsynced_pwm: rd!(r, w, u8),
            fast_pwm_protocol: rd!(r, w, u8),
            motor_pwm_rate: rd!(r, w, u16),
            motor_idle: rd!(r, w, u16),
            gyro_use_32khz: rd!(r, w, u8),
            motor_pwm_inversion: rd!(r, w, u8),
            gyro_to_use: rd!(r, w, u8),
            gyro_high_fsr: rd!(r, w, u8),
            gyro_movement_calib_threshold: rd!(r, w, u8),
            gyro_calib_duration: rd!(r, w, u16),
            gyro_offset_yaw: rd!(r, w, u16),
            gyro_check_overflow: rd!(r, w, u8),
            debug_mode: rd!(r, w, u8),
            debug_mode_count: r.u8().unwrap_or(0),
            ..Default::default()
        };
        a.tail = p[r.pos..].to_vec();
        Ok(a)
    }
    /// `MSP_SET_ADVANCED_CONFIG` (configurator order; ends at debug_mode).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.gyro_sync_denom)
            .u8(self.pid_process_denom)
            .u8(self.use_unsynced_pwm)
            .u8(self.fast_pwm_protocol)
            .u16(self.motor_pwm_rate)
            .u16(self.motor_idle)
            .u8(self.gyro_use_32khz)
            .u8(self.motor_pwm_inversion)
            .u8(self.gyro_to_use)
            .u8(self.gyro_high_fsr)
            .u8(self.gyro_movement_calib_threshold)
            .u16(self.gyro_calib_duration)
            .u16(self.gyro_offset_yaw)
            .u8(self.gyro_check_overflow)
            .u8(self.debug_mode);
        w.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_advanced_roundtrip_146() {
        let api = ApiVersion::new(1, 46);
        let mut a = PidAdvanced::default();
        a.feedforward_roll = 120;
        a.d_max_pitch = 46;
        a.iterm_relax_cutoff = 15;
        a.tpa_breakpoint = 1350;
        a.tail = vec![7, 8, 9];
        let bytes = a.encode(api);
        let b = PidAdvanced::parse(&bytes, api).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn pid_advanced_143_is_shorter() {
        let a = PidAdvanced::default();
        assert!(a.encode(ApiVersion::new(1, 43)).len() < a.encode(ApiVersion::new(1, 46)).len());
    }

    #[test]
    fn filter_config_roundtrip_148() {
        let api = ApiVersion::new(1, 48);
        let mut f = FilterConfig::default();
        f.gyro_lowpass_hz = 300; // > 255: only the 16-bit field must carry it
        f.dyn_notch_count = 3;
        f.dyn_notch_q = 300;
        f.has_rpm_ext = true;
        f.gyro_rpm_notch_q = 500;
        f.gyro_rpm_notch_weights = [100, 100, 100];
        let g = FilterConfig::parse(&f.encode(api), api).unwrap();
        assert_eq!(f, g);
    }

    #[test]
    fn simplified_roundtrip() {
        let mut s = SimplifiedTuning::default();
        s.pids_mode = 2;
        s.gyro_lowpass_dyn_max_hz = 500;
        let t = SimplifiedTuning::parse(&s.encode()).unwrap();
        assert_eq!(s, t);
        assert_eq!(s.encode().len(), 9 + 8 + 10 + 8 + 10 + 8);
    }

    #[test]
    fn fc_version_calendar() {
        let payload = [
            10u8, 0, 0, 9, b'2', b'0', b'2', b'5', b'.', b'1', b'2', b'.', b'1',
        ];
        assert_eq!(parse_fc_version(&payload).unwrap(), "2025.12.1");
        assert_eq!(parse_fc_version(&[4, 5, 1]).unwrap(), "4.5.1");
    }
}
