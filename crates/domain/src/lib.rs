//! Firmware-agnostic domain types shared by every crate.
//! No logic lives here — only data and small accessors.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod tune;
pub use tune::*;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Content hash (blake3, hex) of the source log bytes + session index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Firmware {
    Betaflight {
        version: String,
        /// MSP API version (major, minor), e.g. (1, 46). Zero if unknown.
        api: (u8, u8),
    },
    ArduCopter {
        version: String,
    },
    Unknown {
        product: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Roll = 0,
    Pitch = 1,
    Yaw = 2,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Roll, Axis::Pitch, Axis::Yaw];
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        match self {
            Axis::Roll => "Roll",
            Axis::Pitch => "Pitch",
            Axis::Yaw => "Yaw",
        }
    }
}

// ---------------------------------------------------------------------------
// Flight log (uniform time base)
// ---------------------------------------------------------------------------

/// One axis of the rate loop, all series resampled to `FlightLog::fs_hz`.
/// Units: deg/s for setpoint and gyro; PID terms in firmware-native units.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AxisSeries {
    pub setpoint: Vec<f32>,
    pub gyro_filt: Vec<f32>,
    pub gyro_raw: Option<Vec<f32>>,
    pub p: Option<Vec<f32>>,
    pub i: Option<Vec<f32>>,
    pub d: Option<Vec<f32>>,
    pub ff: Option<Vec<f32>>,
    /// Sum of PID terms (BF) or mixer output (AP `RATE.*Out`).
    pub pid_sum: Option<Vec<f32>>,
    /// ArduPilot only: `PIDx.SRate` slew rate used by Quicktune oscillation detection.
    pub srate: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogMeta {
    pub craft_name: Option<String>,
    /// PID loop rate of the firmware (Hz), if known.
    pub loop_hz: Option<f64>,
    /// Native logging rate before resampling (Hz).
    pub source_rate_hz: f64,
    pub debug_mode: Option<String>,
    pub duration_s: f64,
    /// Index of this session inside a multi-session flash dump.
    pub session_index: usize,
    pub session_count: usize,
    /// Free-form header key/values (BF header lines, AP PARM excluded).
    pub headers: BTreeMap<String, String>,
    /// Non-fatal issues discovered while ingesting.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightLog {
    pub id: LogId,
    pub firmware: Firmware,
    /// Uniform sample rate of every series in this struct.
    pub fs_hz: f64,
    /// Seconds from start, len N.
    pub t: Vec<f32>,
    pub axes: [AxisSeries; 3],
    /// Per motor, normalized 0..1.
    pub motors: Vec<Vec<f32>>,
    /// Normalized 0..1.
    pub throttle: Vec<f32>,
    /// Per motor eRPM if bidirectional DShot / ESC telemetry available.
    pub erpm: Option<Vec<Vec<f32>>>,
    /// Intervals (start_s, end_s) where source data was missing and was interpolated.
    pub gaps: Vec<(f32, f32)>,
    pub meta: LogMeta,
    /// Tune parsed from the log itself (BF header / AP PARM).
    pub tune_at_log: Tune,
}

impl FlightLog {
    pub fn len(&self) -> usize {
        self.t.len()
    }
    pub fn is_empty(&self) -> bool {
        self.t.is_empty()
    }
    pub fn axis(&self, a: Axis) -> &AxisSeries {
        &self.axes[a.index()]
    }
    pub fn duration_s(&self) -> f64 {
        self.t.last().copied().unwrap_or(0.0) as f64
    }
}

// ---------------------------------------------------------------------------
// Analysis outputs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepVariant {
    /// PIDtoolbox `PTstepcalc`: 2 s segments, 500 ms window, 1e-4 regularization.
    PtStep,
    /// PID-Analyzer / ArduPilot PIDReview: 25 Hz Gaussian regularization.
    PidAnalyzer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResponse {
    pub axis: Axis,
    pub variant: StepVariant,
    /// Time axis in ms, 0..=500.
    pub t_ms: Vec<f32>,
    pub mean: Vec<f32>,
    pub p10: Vec<f32>,
    pub p90: Vec<f32>,
    pub n_segments: usize,
    pub rejected: usize,
    /// Peak of mean response inside the first 150 ms (1.0 = no overshoot).
    pub overshoot: f32,
    /// Time (ms) at which the mean first crosses 0.5.
    pub latency_ms: f32,
    /// Time (ms) after which the mean stays within ±5 % of steady state, if reached.
    pub settle_ms: Option<f32>,
    /// Mean of the last 100 ms.
    pub steady_state: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumKind {
    GyroRaw,
    GyroFilt,
    DTerm,
    Setpoint,
    PidSum,
    /// Predicted post-filter spectrum for a candidate filter chain.
    Predicted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spectrum {
    pub axis: Axis,
    pub kind: SpectrumKind,
    pub f_hz: Vec<f32>,
    pub psd_db: Vec<f32>,
    pub nfft: usize,
    pub fs_hz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spectrogram {
    pub axis: Axis,
    pub kind: SpectrumKind,
    pub f_hz: Vec<f32>,
    /// Throttle bin centers, percent 0..=100.
    pub throttle_bins: Vec<f32>,
    /// Row-major: `db[bin * f_hz.len() + fi]`.
    pub db: Vec<f32>,
    /// Number of segments that contributed to each throttle bin.
    pub counts: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoiseBand {
    /// < 100 Hz: intentional motion, propwash, PID oscillation.
    Control,
    /// 100–250 Hz: frame resonance.
    Frame,
    /// > 250 Hz: motor / prop noise.
    Motor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoisePeak {
    pub axis: Axis,
    pub kind: SpectrumKind,
    pub f_hz: f32,
    pub psd_db: f32,
    /// Prominence above local floor in dB.
    pub prominence_db: f32,
    pub band: NoiseBand,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogQuality {
    pub fs_hz: f64,
    pub duration_s: f64,
    pub has_gyro_raw: bool,
    pub has_pid_terms: bool,
    /// Seconds spent hovering: throttle within ±8 % of the modal airborne throttle.
    pub hover_seconds: f64,
    /// Modal airborne throttle in percent (the quad's hover point).
    pub hover_throttle_pct: f64,
    /// Airborne portion of the log (seconds), used to exclude arming/landing.
    pub airborne_range_s: Option<(f32, f32)>,
    /// Percent of samples where any motor is ≥ 98 %.
    pub motor_saturation_pct: f64,
    pub max_setpoint_per_axis: [f32; 3],
    /// Number of usable step segments per axis (as counted by the step estimator).
    pub step_segments_per_axis: [usize; 3],
    pub gap_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisBundle {
    pub log: LogId,
    pub quality: LogQuality,
    pub steps: Vec<StepResponse>,
    pub spectra: Vec<Spectrum>,
    pub spectrograms: Vec<Spectrogram>,
    pub peaks: Vec<NoisePeak>,
}

// ---------------------------------------------------------------------------
// Recommendations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceRef {
    Peak { axis: Axis, f_hz: f32, psd_db: f32 },
    Step { axis: Axis, overshoot: f32, latency_ms: f32 },
    Quality { field: String, value: f64 },
    Text { note: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: uuid::Uuid,
    pub param: ParamRef,
    pub old: ParamValue,
    pub new: ParamValue,
    pub reason: String,
    pub evidence: Vec<EvidenceRef>,
    pub confidence: Confidence,
    pub requires_reboot: bool,
    pub accepted: bool,
}
