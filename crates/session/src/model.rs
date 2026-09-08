use chrono::{DateTime, Utc};
use domain::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Connect,
    Preflight,
    FlightA,
    ImportA,
    FilterAnalysis,
    ApplyFilters,
    FlightB,
    ImportB,
    PidAnalysis,
    ApplyPids,
    FlightC,
    ImportC,
    Compare,
    Report,
}

impl Step {
    pub const ALL: [Step; 14] = [
        Step::Connect,
        Step::Preflight,
        Step::FlightA,
        Step::ImportA,
        Step::FilterAnalysis,
        Step::ApplyFilters,
        Step::FlightB,
        Step::ImportB,
        Step::PidAnalysis,
        Step::ApplyPids,
        Step::FlightC,
        Step::ImportC,
        Step::Compare,
        Step::Report,
    ];

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap()
    }
    pub fn next(self) -> Option<Step> {
        Step::ALL.get(self.index() + 1).copied()
    }
    pub fn prev(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Step::ALL[i])
    }
    pub fn title(self) -> &'static str {
        match self {
            Step::Connect => "Connect flight controller",
            Step::Preflight => "Preflight logging setup",
            Step::FlightA => "Flight A — hover & wobble",
            Step::ImportA => "Import log A",
            Step::FilterAnalysis => "Filter analysis",
            Step::ApplyFilters => "Apply filters",
            Step::FlightB => "Flight B — stick steps",
            Step::ImportB => "Import log B",
            Step::PidAnalysis => "PID analysis",
            Step::ApplyPids => "Apply PIDs",
            Step::FlightC => "Flight C — verification",
            Step::ImportC => "Import log C",
            Step::Compare => "Before / after",
            Step::Report => "Report",
        }
    }
    /// Steps that need a connected flight controller.
    pub fn needs_fc(self) -> bool {
        matches!(
            self,
            Step::Connect | Step::Preflight | Step::ApplyFilters | Step::ApplyPids
        )
    }
    pub fn flight(self) -> Option<Flight> {
        match self {
            Step::FlightA | Step::ImportA => Some(Flight::A),
            Step::FlightB | Step::ImportB => Some(Flight::B),
            Step::FlightC | Step::ImportC => Some(Flight::C),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Locked,
    Active,
    Passed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flight {
    A,
    B,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// FC connected over USB: settings are written by the app.
    Online,
    /// Logs arrive as files (e.g. sent by the customer); settings are handed
    /// over as CLI text / .param file and the tuner confirms they were applied.
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PidStrategy {
    /// Our step-response heuristics (both firmwares).
    #[default]
    Heuristic,
    /// ArduPilot: the pilot flies AUTOTUNE; we verify the result before/after.
    Autotune,
}

/// Where the PID-phase measurement of Flight B comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PidSource {
    /// Stick-step Wiener deconvolution (default).
    #[default]
    StepResponse,
    /// Betaflight CHIRP sweeps → closed-loop frequency response.
    Chirp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    Filters,
    Pids,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightRecord {
    /// Copy of the original file inside the session directory.
    pub log_file: String,
    pub original_path: String,
    pub session_index: usize,
    pub log_id: LogId,
    pub imported_at: DateTime<Utc>,
    pub firmware: Firmware,
    pub craft_name: Option<String>,
    pub fs_hz: f64,
    pub duration_s: f64,
    pub quality: LogQuality,
    pub warnings: Vec<String>,
    /// Anomalies found by the analysis (desync, clipping, oscillation, …).
    #[serde(default)]
    pub anomalies: Vec<Anomaly>,
    /// Tune parsed from the log header.
    pub tune: Tune,
    /// Cached analysis bundle path (relative to the session dir).
    pub bundle_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Override {
    pub step: Step,
    pub guard_id: String,
    pub reason: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRecord {
    pub phase: ApplyPhase,
    pub at: DateTime<Utc>,
    /// Recommendations that were accepted and written.
    pub applied: Vec<Recommendation>,
    /// Online: every field read back equal to what was written.
    /// Offline: tuner confirmed the settings were applied by hand.
    pub verified: bool,
    pub method: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneSnapshot {
    pub label: String,
    pub at: DateTime<Utc>,
    pub file: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub mode: Mode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current: Step,
    pub status: BTreeMap<Step, StepStatus>,
    pub overrides: Vec<Override>,
    pub flight_done: BTreeMap<Flight, DateTime<Utc>>,
    pub flights: BTreeMap<Flight, FlightRecord>,
    pub recs_filters: Vec<Recommendation>,
    pub recs_pids: Vec<Recommendation>,
    pub applies: Vec<ApplyRecord>,
    pub snapshots: Vec<TuneSnapshot>,
    /// Last tune read from the FC (online) — used for "log matches FC" guards.
    pub fc_tune: Option<Tune>,
    pub firmware: Option<Firmware>,
    pub notes: String,
    pub report_file: Option<String>,
    #[serde(default)]
    pub pid_strategy: PidStrategy,
    #[serde(default)]
    pub pid_source: PidSource,
}

impl Session {
    pub fn new(name: String, mode: Mode) -> Self {
        let now = Utc::now();
        let mut status = BTreeMap::new();
        for s in Step::ALL {
            status.insert(s, StepStatus::Locked);
        }
        let first = match mode {
            Mode::Online => Step::Connect,
            Mode::Offline => {
                status.insert(Step::Connect, StepStatus::Skipped);
                status.insert(Step::Preflight, StepStatus::Skipped);
                Step::FlightA
            }
        };
        status.insert(first, StepStatus::Active);
        Self {
            schema_version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name,
            mode,
            created_at: now,
            updated_at: now,
            current: first,
            status,
            overrides: Vec::new(),
            flight_done: BTreeMap::new(),
            flights: BTreeMap::new(),
            recs_filters: Vec::new(),
            recs_pids: Vec::new(),
            applies: Vec::new(),
            snapshots: Vec::new(),
            fc_tune: None,
            firmware: None,
            notes: String::new(),
            report_file: None,
            pid_strategy: PidStrategy::default(),
            pid_source: PidSource::default(),
        }
    }

    pub fn is_overridden(&self, step: Step, guard_id: &str) -> bool {
        self.overrides
            .iter()
            .any(|o| o.step == step && o.guard_id == guard_id)
    }

    pub fn apply_for(&self, phase: ApplyPhase) -> Option<&ApplyRecord> {
        self.applies.iter().rev().find(|a| a.phase == phase)
    }

    pub fn recs(&self, phase: ApplyPhase) -> &Vec<Recommendation> {
        match phase {
            ApplyPhase::Filters => &self.recs_filters,
            ApplyPhase::Pids => &self.recs_pids,
        }
    }

    pub fn recs_mut(&mut self, phase: ApplyPhase) -> &mut Vec<Recommendation> {
        match phase {
            ApplyPhase::Filters => &mut self.recs_filters,
            ApplyPhase::Pids => &mut self.recs_pids,
        }
    }
}

/// What the UI renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session: Session,
    pub steps: Vec<StepView>,
    pub can_next: bool,
    pub can_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepView {
    pub step: Step,
    pub title: String,
    pub status: StepStatus,
    pub needs_fc: bool,
    pub guards: Vec<crate::guards::GuardResult>,
}
