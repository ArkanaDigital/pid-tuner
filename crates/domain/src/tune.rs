//! Tune (PID + filter configuration) representations for each firmware.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "firmware", rename_all = "snake_case")]
pub enum Tune {
    Bf(BfTune),
    Ap(ApTune),
    Unknown,
}

/// A single parameter reference, firmware-specific.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "firmware", content = "name", rename_all = "snake_case")]
pub enum ParamRef {
    /// Betaflight CLI setting name, e.g. `d_roll`, `gyro_lpf1_static_hz`.
    Bf(String),
    /// ArduPilot parameter name, e.g. `ATC_RAT_RLL_FLTD`.
    Ap(String),
}

impl ParamRef {
    pub fn name(&self) -> &str {
        match self {
            ParamRef::Bf(s) | ParamRef::Ap(s) => s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ParamValue {
    F32(f32),
    I32(i32),
    U8(u8),
    U16(u16),
    Bool(bool),
    /// Enumerated CLI value (e.g. `PT1`, `OFF`, `RPY`).
    Enum(u8),
}

impl ParamValue {
    pub fn as_f64(&self) -> f64 {
        match *self {
            ParamValue::F32(v) => v as f64,
            ParamValue::I32(v) => v as f64,
            ParamValue::U8(v) => v as f64,
            ParamValue::U16(v) => v as f64,
            ParamValue::Bool(v) => v as u8 as f64,
            ParamValue::Enum(v) => v as f64,
        }
    }
}

impl std::fmt::Display for ParamValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamValue::F32(v) => write!(f, "{v}"),
            ParamValue::I32(v) => write!(f, "{v}"),
            ParamValue::U8(v) => write!(f, "{v}"),
            ParamValue::U16(v) => write!(f, "{v}"),
            ParamValue::Bool(v) => write!(f, "{}", if *v { "ON" } else { "OFF" }),
            ParamValue::Enum(v) => write!(f, "{v}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Betaflight
// ---------------------------------------------------------------------------

/// P/I/D/FF/D_max for one axis (Betaflight units).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BfAxisPid {
    pub p: u8,
    pub i: u8,
    pub d: u8,
    pub ff: u16,
    pub d_max: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum BfFilterType {
    Pt1,
    Biquad,
    Pt2,
    Pt3,
}

impl BfFilterType {
    pub fn from_cli(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PT1" | "0" => Some(Self::Pt1),
            "BIQUAD" | "1" => Some(Self::Biquad),
            "PT2" | "2" => Some(Self::Pt2),
            "PT3" | "3" => Some(Self::Pt3),
            _ => None,
        }
    }
    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Pt1 => "PT1",
            Self::Biquad => "BIQUAD",
            Self::Pt2 => "PT2",
            Self::Pt3 => "PT3",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BfFilterConfig {
    pub gyro_lpf1_static_hz: u16,
    pub gyro_lpf1_type: BfFilterType,
    pub gyro_lpf1_dyn_min_hz: u16,
    pub gyro_lpf1_dyn_max_hz: u16,
    pub gyro_lpf2_static_hz: u16,
    pub gyro_lpf2_type: BfFilterType,
    pub gyro_notch1_hz: u16,
    pub gyro_notch1_cutoff: u16,
    pub gyro_notch2_hz: u16,
    pub gyro_notch2_cutoff: u16,
    pub dterm_lpf1_static_hz: u16,
    pub dterm_lpf1_type: BfFilterType,
    pub dterm_lpf1_dyn_min_hz: u16,
    pub dterm_lpf1_dyn_max_hz: u16,
    pub dterm_lpf2_static_hz: u16,
    pub dterm_lpf2_type: BfFilterType,
    pub dterm_notch_hz: u16,
    pub dterm_notch_cutoff: u16,
    pub dyn_notch_count: u8,
    pub dyn_notch_q: u16,
    pub dyn_notch_min_hz: u16,
    pub dyn_notch_max_hz: u16,
    pub rpm_filter_harmonics: u8,
    pub rpm_filter_min_hz: u16,
    pub rpm_filter_q: u16,
    pub rpm_filter_fade_range_hz: u16,
    pub rpm_filter_lpf_hz: u16,
}

impl Default for BfFilterConfig {
    /// Betaflight 4.5 / 2025.12 defaults.
    fn default() -> Self {
        Self {
            gyro_lpf1_static_hz: 250,
            gyro_lpf1_type: BfFilterType::Pt1,
            gyro_lpf1_dyn_min_hz: 250,
            gyro_lpf1_dyn_max_hz: 500,
            gyro_lpf2_static_hz: 500,
            gyro_lpf2_type: BfFilterType::Pt1,
            gyro_notch1_hz: 0,
            gyro_notch1_cutoff: 0,
            gyro_notch2_hz: 0,
            gyro_notch2_cutoff: 0,
            dterm_lpf1_static_hz: 75,
            dterm_lpf1_type: BfFilterType::Pt1,
            dterm_lpf1_dyn_min_hz: 75,
            dterm_lpf1_dyn_max_hz: 150,
            dterm_lpf2_static_hz: 150,
            dterm_lpf2_type: BfFilterType::Pt1,
            dterm_notch_hz: 0,
            dterm_notch_cutoff: 0,
            dyn_notch_count: 3,
            dyn_notch_q: 300,
            dyn_notch_min_hz: 100,
            dyn_notch_max_hz: 600,
            rpm_filter_harmonics: 3,
            rpm_filter_min_hz: 100,
            rpm_filter_q: 500,
            rpm_filter_fade_range_hz: 50,
            rpm_filter_lpf_hz: 150,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BfSimplified {
    /// `simplified_pids_mode`: 0 OFF, 1 RP, 2 RPY.
    pub pids_mode: u8,
    pub master_multiplier: u8,
    pub pi_gain: u8,
    pub i_gain: u8,
    pub d_gain: u8,
    pub d_max_gain: u8,
    pub feedforward_gain: u8,
    pub pitch_pi_gain: u8,
    pub pitch_d_gain: u8,
    pub gyro_filter: bool,
    pub gyro_filter_multiplier: u8,
    pub dterm_filter: bool,
    pub dterm_filter_multiplier: u8,
}

impl Default for BfSimplified {
    fn default() -> Self {
        Self {
            pids_mode: 2,
            master_multiplier: 100,
            pi_gain: 100,
            i_gain: 100,
            d_gain: 100,
            d_max_gain: 100,
            feedforward_gain: 100,
            pitch_pi_gain: 100,
            pitch_d_gain: 100,
            gyro_filter: true,
            gyro_filter_multiplier: 100,
            dterm_filter: true,
            dterm_filter_multiplier: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BfTune {
    /// Roll, Pitch, Yaw.
    pub pids: [BfAxisPid; 3],
    pub d_max_gain: u8,
    pub d_max_advance: u8,
    pub anti_gravity_gain: u16,
    pub iterm_relax: u8,
    pub iterm_relax_type: u8,
    pub iterm_relax_cutoff: u8,
    pub feedforward_smooth_factor: u8,
    pub feedforward_jitter_factor: u8,
    pub feedforward_boost: u8,
    pub tpa_rate: u8,
    pub tpa_breakpoint: u16,
    pub filters: BfFilterConfig,
    pub simplified: BfSimplified,
    /// Every `set name = value` line seen in the header / CLI, raw.
    pub raw: BTreeMap<String, String>,
    /// MSP API version (major, minor); (0,0) if read from a log header only.
    pub api: (u8, u8),
}

impl Default for BfTune {
    fn default() -> Self {
        Self {
            pids: [
                BfAxisPid { p: 45, i: 80, d: 30, ff: 120, d_max: 40 },
                BfAxisPid { p: 47, i: 84, d: 34, ff: 125, d_max: 46 },
                BfAxisPid { p: 45, i: 80, d: 0, ff: 120, d_max: 0 },
            ],
            d_max_gain: 37,
            d_max_advance: 20,
            anti_gravity_gain: 80,
            iterm_relax: 1,
            iterm_relax_type: 0,
            iterm_relax_cutoff: 15,
            feedforward_smooth_factor: 65,
            feedforward_jitter_factor: 7,
            feedforward_boost: 15,
            tpa_rate: 65,
            tpa_breakpoint: 1350,
            filters: BfFilterConfig::default(),
            simplified: BfSimplified::default(),
            raw: BTreeMap::new(),
            api: (0, 0),
        }
    }
}

impl BfTune {
    pub fn get_raw(&self, key: &str) -> Option<&str> {
        self.raw.get(key).map(String::as_str)
    }

    /// Betaflight ≤ 4.5 names the per-axis D gains the old way: CLI `d_roll`
    /// is **D Max** and `d_min_roll` is the "Derivative" column. From 2025.12
    /// (API ≥ 1.47) `d_roll` is Derivative and `d_max_roll` is D Max.
    pub fn legacy_d_naming(&self) -> bool {
        if self.api != (0, 0) {
            return self.api < (1, 47);
        }
        match self.firmware_version() {
            Some((major, minor)) => major < 4 || (major == 4 && minor < 6),
            None => false,
        }
    }

    /// (major, minor) from the `Firmware revision` header, if present.
    pub fn firmware_version(&self) -> Option<(u32, u32)> {
        let rev = self.raw.get("Firmware revision")?;
        let v = rev.split_whitespace().nth(1)?;
        let mut it = v.split('.');
        Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    }

    /// CLI name of the "Derivative" gain for an axis (0 roll, 1 pitch, 2 yaw).
    pub fn cli_d_name(&self, axis: usize) -> String {
        let ax = ["roll", "pitch", "yaw"][axis.min(2)];
        if self.legacy_d_naming() { format!("d_min_{ax}") } else { format!("d_{ax}") }
    }

    /// CLI name of the "D Max" gain for an axis.
    pub fn cli_d_max_name(&self, axis: usize) -> String {
        let ax = ["roll", "pitch", "yaw"][axis.min(2)];
        if self.legacy_d_naming() { format!("d_{ax}") } else { format!("d_max_{ax}") }
    }
}

// ---------------------------------------------------------------------------
// ArduPilot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MavParamType {
    Int8,
    Int16,
    Int32,
    Real32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApTune {
    pub params: BTreeMap<String, f32>,
    pub types: BTreeMap<String, MavParamType>,
}

impl ApTune {
    pub fn get(&self, name: &str) -> Option<f32> {
        self.params.get(name).copied()
    }
}
