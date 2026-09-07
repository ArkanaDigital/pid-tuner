//! Rule engine: (tune, analysis) → recommendations with evidence.
//!
//! Rules are deliberately conservative: every change is small, carries a
//! reason and a confidence, and nothing is applied automatically.

pub mod bf;

use domain::{AnalysisBundle, Recommendation, Tune};

/// Which wizard phase the recommendations are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Filters,
    Pids,
}

pub fn recommend(tune: &Tune, bundle: &AnalysisBundle, phase: Phase) -> Vec<Recommendation> {
    match tune {
        Tune::Bf(t) => match phase {
            Phase::Filters => bf::filters(t, bundle),
            Phase::Pids => bf::pids(t, bundle),
        },
        Tune::Ap(_) => Vec::new(), // ArduPilot rules land in Phase 5.
        Tune::Unknown => Vec::new(),
    }
}
