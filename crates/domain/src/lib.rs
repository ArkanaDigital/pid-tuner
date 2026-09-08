//! Firmware-agnostic domain types shared by every crate.
//! No logic lives here — only data and small accessors.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod ap_consts;
pub mod fc;
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
    /// Measured logging rate per message type (ArduPilot: RATE, PIDR, IMU, ISBD, …).
    #[serde(default)]
    pub msg_rates_hz: BTreeMap<String, f64>,
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
    /// High-rate gyro tracks that cannot live on the uniform grid (ArduPilot
    /// IMU batch sampler): bursts of samples at their own rate.
    #[serde(default)]
    pub gyro_hr: Vec<RawGyroTrack>,
    /// Betaflight `debug[k]` fields on the uniform grid (nearest sample), if logged.
    #[serde(default)]
    pub debug: Vec<Vec<f32>>,
    /// Betaflight CHIRP system-identification sweeps found in the log.
    #[serde(default)]
    pub chirp: Option<ChirpInfo>,
    /// Betaflight `flightModeFlags` (boxId bitmask from slow frames) per grid sample, if logged.
    #[serde(default)]
    pub flight_mode_flags: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Betaflight CHIRP (system identification)
// ---------------------------------------------------------------------------

/// `chirp_*` CLI settings (betaflight `pid.c` resetPidProfile defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChirpConfig {
    pub lag_freq_hz: f32,
    pub lead_freq_hz: f32,
    /// Roll, pitch, yaw amplitude (°/s).
    pub amplitude: [u16; 3],
    pub f_start_hz: f32,
    pub f_end_hz: f32,
    pub time_s: f32,
}

impl Default for ChirpConfig {
    fn default() -> Self {
        Self {
            lag_freq_hz: 3.0,
            lead_freq_hz: 30.0,
            amplitude: [230, 230, 180],
            f_start_hz: 0.2,
            f_end_hz: 600.0,
            time_s: 20.0,
        }
    }
}

/// How a chirp segment was delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChirpGate {
    /// BOXCHIRP flight-mode flag and `debug[1]` agreed.
    Both,
    /// Only `debug[1]` (no slow frames decoded).
    Debug,
    /// Only the flight-mode flag (debug fields disabled).
    FlightMode,
}

/// One activation of CHIRP mode on one axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChirpSegment {
    pub axis: Axis,
    /// Index range on the uniform grid (`i0..i1`).
    pub i0: usize,
    pub i1: usize,
    pub t0_s: f32,
    pub t1_s: f32,
    #[serde(with = "nan_f32")]
    pub f_start_hz: f32,
    #[serde(with = "nan_f32")]
    pub f_end_hz: f32,
    pub source: ChirpGate,
    /// Flown in ANGLE/HORIZON mode: the rate setpoint then contains the outer
    /// attitude loop, so the sweep does not identify the rate loop alone.
    #[serde(default)]
    pub angle_mode: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChirpInfo {
    pub config: Option<ChirpConfig>,
    pub segments: Vec<ChirpSegment>,
    /// `debug[1..3]` look like DEBUG_CHIRP output regardless of the `debug_mode` id.
    pub debug_is_chirp: bool,
}

impl ChirpInfo {
    pub fn segments_for(&self, axis: Axis) -> impl Iterator<Item = &ChirpSegment> {
        self.segments.iter().filter(move |s| s.axis == axis)
    }
}

/// One phase-margin target of the frequency-response analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrTarget {
    pub pm_deg: f32,
    /// Where the open loop reaches −(180 − pm) degrees (Hz).
    #[serde(with = "nan_f32")]
    pub crossover_hz: f32,
    /// Gain multiplier that would put the crossover there (1/|L|).
    #[serde(with = "nan_f32")]
    pub gain_to_target: f32,
    /// Largest gain in [0.5, 2] keeping the predicted sensitivity peak ≤ 2.
    #[serde(with = "nan_f32")]
    pub gain_for_sens_limit: f32,
}

/// Scalar metrics of one axis' closed-loop frequency response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrMetrics {
    #[serde(with = "nan_f32")]
    pub bandwidth_hz: f32,
    #[serde(with = "nan_f32")]
    pub crossover_hz: f32,
    #[serde(with = "nan_f32")]
    pub phase_margin_deg: f32,
    #[serde(with = "nan_f32")]
    pub max_phase_margin_deg: f32,
    #[serde(with = "nan_f32")]
    pub resonant_peak_db: f32,
    #[serde(with = "nan_f32")]
    pub resonant_peak_hz: f32,
    #[serde(with = "nan_f32")]
    pub loop_delay_ms: f32,
    #[serde(with = "nan_f32")]
    pub low_freq_err_db: f32,
    #[serde(with = "nan_f32")]
    pub coherence_mean: f32,
    /// First bin above 20 Hz where coherence drops below 0.5. Display only.
    #[serde(with = "nan_f32")]
    pub noise_floor_hz: f32,
    #[serde(with = "nan_f32")]
    pub sens_peak_db: f32,
    #[serde(with = "nan_f32")]
    pub sens_peak_hz: f32,
    #[serde(with = "nan_f32")]
    pub step_overshoot: f32,
    #[serde(with = "nan_f32")]
    pub step_rise_ms: f32,
    #[serde(with = "nan_f32")]
    pub step_settle_ms: f32,
    pub targets: Vec<FrTarget>,
}

/// Closed-loop frequency response setpoint → gyro of one axis (Betaflight CHIRP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrequencyResponse {
    pub axis: Axis,
    /// Built from ANGLE/HORIZON-mode sweeps only (rate loop confounded by the attitude loop).
    #[serde(default)]
    pub angle_mode: bool,
    pub f_hz: Vec<f32>,
    #[serde(with = "nan_vec_f32")]
    pub h_mag_db: Vec<f32>,
    #[serde(with = "nan_vec_f32")]
    pub h_phase_deg: Vec<f32>,
    #[serde(with = "nan_vec_f32")]
    pub coherence: Vec<f32>,
    /// Open loop L = H/(1−H), NaN where f < 2 Hz or coherence < 0.5.
    #[serde(with = "nan_vec_f32")]
    pub l_mag_db: Vec<f32>,
    #[serde(with = "nan_vec_f32")]
    pub l_phase_deg: Vec<f32>,
    /// Sensitivity S = 1 − H.
    #[serde(with = "nan_vec_f32")]
    pub s_mag_db: Vec<f32>,
    /// Step response reconstructed from H (0..100 ms).
    pub step_t_ms: Vec<f32>,
    pub step: Vec<f32>,
    pub fs_hz: f64,
    pub segment_size: usize,
    pub n_windows: usize,
    pub n_sweeps: usize,
    pub sweep_seconds: f32,
    pub metrics: FrMetrics,
}

/// One burst of consecutive gyro samples (ArduPilot `ISBH` + its `ISBD` chunks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GyroBatch {
    /// Start time (seconds, same base as `FlightLog::t`).
    pub t0_s: f32,
    /// Roll, pitch, yaw rates in deg/s.
    pub xyz: [Vec<f32>; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawGyroTrack {
    pub fs_hz: f64,
    /// Sensor instance as logged.
    pub instance: u8,
    /// True when this track is post-filter data (`INS_LOG_BAT_OPT` bit 1/2).
    pub post_filter: bool,
    pub batches: Vec<GyroBatch>,
}

impl RawGyroTrack {
    pub fn total_samples(&self) -> usize {
        self.batches.iter().map(|b| b.xyz[0].len()).sum()
    }
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
    /// Time (ms) at which the mean first crosses 0.5 (NaN when no segments; serialised as null).
    #[serde(with = "nan_f32")]
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
    /// ArduPilot: measured PIDx logging rate (None when PIDx absent).
    #[serde(default)]
    pub pid_rate_hz: Option<f64>,
    /// Max |PID output| per axis (ArduPilot `RATE.*Out`, −1..1). None for Betaflight.
    #[serde(default)]
    pub max_pid_out: Option<[f32; 3]>,
    /// Number of high-rate gyro batches available for spectra.
    #[serde(default)]
    pub gyro_hr_batches: usize,
    /// Betaflight CHIRP: sweeps, Welch windows and mean coherence (5–100 Hz) per axis.
    #[serde(default)]
    pub chirp_sweeps_per_axis: [usize; 3],
    #[serde(default)]
    pub chirp_windows_per_axis: [usize; 3],
    #[serde(default)]
    pub chirp_coherence_per_axis: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisBundle {
    pub log: LogId,
    pub quality: LogQuality,
    pub steps: Vec<StepResponse>,
    pub spectra: Vec<Spectrum>,
    pub spectrograms: Vec<Spectrogram>,
    pub peaks: Vec<NoisePeak>,
    /// Flight anomalies found in the log (desync, clipping, oscillation, …).
    #[serde(default)]
    pub anomalies: Vec<Anomaly>,
    /// Closed-loop frequency responses from CHIRP sweeps (empty without chirp data).
    #[serde(default)]
    pub freq_resp: Vec<FrequencyResponse>,
}

// ---------------------------------------------------------------------------
// Anomalies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    /// One motor pinned at maximum while its eRPM collapsed (or, without RPM
    /// telemetry, while the others fell and the quad rolled/pitched hard).
    MotorDesync,
    /// A motor held at 100 % for a sustained time: no control authority left.
    MotorSaturation,
    /// A motor held at the idle floor while airborne: authority lost on the low side.
    MotorFloor,
    /// Gyro reading at the sensor range limit (±2000 °/s class).
    GyroClipping,
    /// Sustained un-commanded oscillation of the gyro around the setpoint.
    Oscillation,
    /// Hover motor outputs differ a lot between motors (CG / bent prop / weak motor).
    MotorImbalance,
    /// One motor's eRPM in hover deviates from the others (prop / bearing / ESC).
    RpmImbalance,
    /// eRPM telemetry reads zero while the motor is commanded on.
    RpmDropout,
    /// Missing log frames (SD/flash too slow, logging rate too high).
    LogGap,
    /// Un-commanded yaw rotation above 1000 °/s (yaw spin / crash).
    YawSpin,
    /// Gyro moves against the setpoint: wrong board orientation or motor direction.
    ControlReversed,
    /// High-frequency raw gyro energy far above normal (bearing / loose prop / frame).
    Vibration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    pub severity: Severity,
    /// Seconds from log start.
    pub t_start_s: f32,
    pub t_end_s: f32,
    pub axis: Option<Axis>,
    /// Motor index (0-based) when the finding concerns one motor.
    pub motor: Option<usize>,
    /// Kind-specific magnitude (°/s, %, Hz, …) — see `detail` for the unit.
    pub value: f32,
    pub detail: String,
}

impl AnomalyKind {
    pub fn title(self) -> &'static str {
        match self {
            AnomalyKind::MotorDesync => "Motor desync",
            AnomalyKind::MotorSaturation => "Motor saturation",
            AnomalyKind::MotorFloor => "Motor at idle floor",
            AnomalyKind::GyroClipping => "Gyro clipping",
            AnomalyKind::Oscillation => "Oscillation",
            AnomalyKind::MotorImbalance => "Motor imbalance",
            AnomalyKind::RpmImbalance => "RPM imbalance",
            AnomalyKind::RpmDropout => "RPM telemetry dropout",
            AnomalyKind::LogGap => "Log gap",
            AnomalyKind::YawSpin => "Yaw spin",
            AnomalyKind::ControlReversed => "Control reversed",
            AnomalyKind::Vibration => "Vibration",
        }
    }
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
    Peak {
        axis: Axis,
        f_hz: f32,
        psd_db: f32,
    },
    Step {
        axis: Axis,
        overshoot: f32,
        latency_ms: f32,
    },
    Quality {
        field: String,
        value: f64,
    },
    Text {
        note: String,
    },
    FreqResp {
        axis: Axis,
        bandwidth_hz: f32,
        phase_margin_deg: f32,
        coherence: f32,
    },
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

/// serde for f32 fields that may be NaN: JSON has no NaN, so `null` ⇄ NaN.
pub mod nan_f32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_f32(*v)
        } else {
            s.serialize_none()
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
        Ok(Option::<f32>::deserialize(d)?.unwrap_or(f32::NAN))
    }
}

/// serde for `Vec<f32>` with NaN/inf elements: each non-finite value ⇄ `null`.
pub mod nan_vec_f32 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let opt: Vec<Option<f32>> = v.iter().map(|x| x.is_finite().then_some(*x)).collect();
        opt.serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        Ok(Vec::<Option<f32>>::deserialize(d)?
            .into_iter()
            .map(|o| o.unwrap_or(f32::NAN))
            .collect())
    }
}
