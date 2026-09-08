//! Rule engine: (tune, analysis) → recommendations with evidence.
//!
//! Rules are deliberately conservative: every change is small, carries a
//! reason and a confidence, and nothing is applied automatically.

pub mod ap;
pub mod ap_param_meta;
pub mod bf;
pub mod bf_chirp;
pub mod bf_param_meta;

use domain::{AnalysisBundle, Recommendation, Tune};

/// Which wizard phase the recommendations are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Filters,
    Pids,
}

pub fn recommend(tune: &Tune, bundle: &AnalysisBundle, phase: Phase) -> Vec<Recommendation> {
    recommend_with(tune, bundle, phase, None, None)
}

/// `hover_thr` (0..1, from `CTUN.ThH`) and `max_srate` (per axis, `PIDx.SRate`)
/// feed the ArduPilot rules; pass `None` when unknown.
pub fn recommend_with(
    tune: &Tune,
    bundle: &AnalysisBundle,
    phase: Phase,
    hover_thr: Option<f32>,
    max_srate: Option<[f32; 3]>,
) -> Vec<Recommendation> {
    match tune {
        Tune::Bf(t) => match phase {
            Phase::Filters => bf::filters(t, bundle),
            Phase::Pids => bf::pids(t, bundle),
        },
        Tune::Ap(t) => match phase {
            Phase::Filters => ap::filters(t, bundle, hover_thr),
            Phase::Pids => ap::pids(t, bundle, max_srate),
        },
        Tune::Unknown => Vec::new(),
    }
}

/// Convenience: derive the ArduPilot inputs from the log itself.
pub fn recommend_for_log(
    log: &domain::FlightLog,
    bundle: &AnalysisBundle,
    phase: Phase,
) -> Vec<Recommendation> {
    let hover = log
        .meta
        .headers
        .get("ap.hover_thr")
        .and_then(|v| v.parse::<f32>().ok());
    let srate = ap::max_srate(log, bundle.quality.airborne_range_s);
    recommend_with(&log.tune_at_log, bundle, phase, hover, srate)
}
