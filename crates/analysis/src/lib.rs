//! Turns a [`FlightLog`] into step responses, spectra, spectrograms, noise
//! peaks and a quality summary. Pure computation; parallel per axis.

pub mod peaks;
pub mod quality;
pub mod spectrogram;
pub mod spectrum;
pub mod step_response;

use domain::{AnalysisBundle, Axis, FlightLog, SpectrumKind};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use peaks::PeakOpts;
pub use spectrogram::SpectrogramOpts;
pub use spectrum::{SpectrumMode, SpectrumOpts};
pub use step_response::StepOpts;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisOpts {
    pub step: StepOpts,
    pub spectrum: SpectrumOpts,
    pub spectrogram: SpectrogramOpts,
    pub peaks: PeakOpts,
    /// Optional time range (seconds) to restrict spectrum/spectrogram analysis.
    pub range_s: Option<(f32, f32)>,
}

/// Run the whole pipeline. Progress is reported as a fraction in `0..=1`.
pub fn analyze(log: &FlightLog, opts: &AnalysisOpts, progress: impl Fn(f32) + Sync) -> AnalysisBundle {
    progress(0.0);
    // Spectra/spectrograms default to the airborne part of the log so arming
    // spin-up and touchdown do not produce phantom peaks.
    let range = opts.range_s.or_else(|| quality::airborne_range(log));
    let opts = AnalysisOpts { range_s: range, ..opts.clone() };
    let opts = &opts;
    let steps: Vec<_> = Axis::ALL
        .par_iter()
        .map(|&a| step_response::step_response(log, a, &opts.step))
        .collect();
    progress(0.35);

    let kinds = [SpectrumKind::GyroRaw, SpectrumKind::GyroFilt, SpectrumKind::DTerm];
    let jobs: Vec<(Axis, SpectrumKind)> = Axis::ALL
        .iter()
        .flat_map(|a| kinds.iter().map(move |k| (*a, *k)))
        .collect();
    let spectra: Vec<_> = jobs
        .par_iter()
        .filter_map(|(a, k)| spectrum::spectrum(log, *a, *k, &opts.spectrum, opts.range_s))
        .collect();
    progress(0.6);

    let spectrograms: Vec<_> = Axis::ALL
        .par_iter()
        .filter_map(|&a| {
            let kind = if log.axis(a).gyro_raw.is_some() {
                SpectrumKind::GyroRaw
            } else {
                SpectrumKind::GyroFilt
            };
            spectrogram::throttle_spectrogram(log, a, kind, &opts.spectrogram, opts.range_s)
        })
        .collect();
    progress(0.85);

    let peaks: Vec<_> = spectra
        .iter()
        .flat_map(|s| peaks::find_peaks(s, &opts.peaks))
        .collect();

    let mut quality = quality::log_quality(log);
    for s in &steps {
        quality.step_segments_per_axis[s.axis.index()] = s.n_segments;
    }
    progress(1.0);

    AnalysisBundle {
        log: log.id.clone(),
        quality,
        steps,
        spectra,
        spectrograms,
        peaks,
    }
}

/// Index range of `log.t` covering `range_s` (whole log if `None`).
pub(crate) fn range_idx(log: &FlightLog, range_s: Option<(f32, f32)>) -> (usize, usize) {
    match range_s {
        None => (0, log.len()),
        Some((t0, t1)) => dsp::decimate::range_indices(&log.t, t0, t1),
    }
}
