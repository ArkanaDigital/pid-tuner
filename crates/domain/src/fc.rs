//! Firmware-agnostic flight-controller interface shared by `fc-msp` and
//! `fc-mavlink`, plus the status/result types the wizard and UI consume.

use crate::{Firmware, Recommendation, Tune};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FcKind {
    Msp,
    Mavlink,
}

#[derive(Debug, thiserror::Error)]
pub enum FcError {
    #[error("not connected")]
    NotConnected,
    #[error("refusing to write: {0}")]
    Refused(String),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

/// Live flight-controller status. Filled by the backend, evaluated by guards.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FcStatus {
    pub connected: bool,
    pub port: Option<String>,
    pub kind: Option<FcKind>,
    pub firmware: Option<Firmware>,
    pub armed: bool,
    pub heartbeat_age_s: f32,
    pub tune: Option<Tune>,
    /// BF: effective blackbox rate. AP: PIDx logging rate implied by LOG_BITMASK (loop rate or 10 Hz).
    pub log_rate_hz: Option<f64>,
    /// BF only.
    pub debug_mode: Option<String>,
    pub storage_free_bytes: Option<u64>,
    /// AP: LOG_BITMASK bits 0 (ATTITUDE_FAST) and 12 (PID) set. BF: always true.
    pub pid_logging_enabled: Option<bool>,
    /// AP: batch sampler / raw logging configured. BF: gyroUnfilt native or debug GYRO_SCALED.
    pub raw_gyro_logging_enabled: Option<bool>,
    pub snapshot_taken: bool,
    // ---- ArduPilot ----
    pub log_bitmask: Option<u32>,
    pub batch_configured: Option<bool>,
    pub loop_rate_hz: Option<f64>,
    pub autotune_axes: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub param: String,
    pub wanted: String,
    pub read_back: Option<String>,
    pub ok: bool,
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub outcomes: Vec<ApplyOutcome>,
    pub verified: bool,
    /// The FC rebooted as part of the apply (caller must reconnect / has reconnected).
    pub rebooted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub label: String,
    /// "txt" (BF diff all / MSP json) or "param" (AP NAME,VALUE)
    pub ext: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: u32,
    pub size: u64,
    pub time_utc: Option<u64>,
}

/// What the Preflight step is allowed to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightFix {
    /// BF: blackbox rate ≥ 2 kHz (+ GYRO_SCALED on ≤ 4.3). AP: LOG_BITMASK bits 0+12(+19) and batch sampler.
    Logging,
}

pub type ProgressFn<'a> = &'a mut dyn FnMut(u64, u64);

pub trait FlightController: Send {
    fn kind(&self) -> FcKind;
    /// Cheap heartbeat/armed poll.
    fn poll(&mut self) -> Result<FcStatus, FcError>;
    /// Full read: identity, tune, logging config, storage.
    fn full_status(&mut self) -> Result<FcStatus, FcError>;
    fn read_tune(&mut self) -> Result<Tune, FcError>;
    fn backup(&mut self) -> Result<Backup, FcError>;
    fn preflight_fix(&mut self, fix: PreflightFix) -> Result<ApplyResult, FcError>;
    fn apply(&mut self, recs: &[Recommendation]) -> Result<ApplyResult, FcError>;
    fn reboot_and_reconnect(&mut self) -> Result<(), FcError>;
    fn list_logs(&mut self) -> Result<Vec<LogEntry>, FcError>;
    /// `id = None` → the most recent log.
    fn download_log(&mut self, id: Option<u32>, progress: ProgressFn<'_>, cancel: &AtomicBool) -> Result<Vec<u8>, FcError>;
    /// Human-readable text the pilot can apply by hand (BF CLI `set` lines / AP `NAME,VALUE`).
    fn export_text(&self, recs: &[Recommendation]) -> String;
}

/// Firmware-specific text export usable without a live FC.
pub fn export_text_for(firmware: Option<&Firmware>, recs: &[Recommendation]) -> String {
    let accepted: Vec<&Recommendation> = recs.iter().filter(|r| r.accepted).collect();
    if accepted.is_empty() {
        return String::new();
    }
    match firmware {
        Some(Firmware::ArduCopter { .. }) => {
            let mut s = String::from("# PID Tuner — ArduPilot parameters (load with Mission Planner / QGC)\n");
            for r in accepted {
                s.push_str(&format!("{},{}\n", r.param.name(), r.new));
            }
            s
        }
        _ => {
            let mut s = String::from("# PID Tuner\n");
            for r in accepted {
                s.push_str(&format!("set {} = {}\n", r.param.name(), r.new));
            }
            s.push_str("save\n");
            s
        }
    }
}
