//! Turns a [`FlightLog`] into step responses, spectra, spectrograms, noise
//! peaks and a quality summary. Pure computation; parallel per axis.

pub mod anomaly;
pub mod chirp;
pub mod hr;
pub mod peaks;
pub mod predicted;
pub mod quality;
pub mod spectrogram;
pub mod spectrum;
pub mod step_response;

use domain::{AnalysisBundle, Axis, FlightLog, SpectrumKind};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use anomaly::AnomalyOpts;
pub use chirp::ChirpOpts;
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
    pub anomaly: AnomalyOpts,
    pub chirp: ChirpOpts,
    /// Optional time range (seconds) to restrict spectrum/spectrogram analysis.
    pub range_s: Option<(f32, f32)>,
}

/// Run the whole pipeline. Progress is reported as a fraction in `0..=1`.
pub fn analyze(
    log: &FlightLog,
    opts: &AnalysisOpts,
    progress: impl Fn(f32) + Sync,
) -> AnalysisBundle {
    progress(0.0);
    // Spectra/spectrograms default to the airborne part of the log so arming
    // spin-up and touchdown do not produce phantom peaks.
    let range = opts.range_s.or_else(|| quality::airborne_range(log));
    let mut opts = AnalysisOpts {
        range_s: range,
        ..opts.clone()
    };
    if matches!(log.firmware, domain::Firmware::ArduCopter { .. })
        && opts.step.min_input_dps == StepOpts::default().min_input_dps
    {
        opts.step = StepOpts::ardupilot();
    }
    let opts = &opts;
    let steps: Vec<_> = Axis::ALL
        .par_iter()
        .map(|&a| step_response::step_response(log, a, &opts.step))
        .collect();
    progress(0.35);

    let kinds = [
        SpectrumKind::GyroRaw,
        SpectrumKind::GyroFilt,
        SpectrumKind::DTerm,
    ];
    let jobs: Vec<(Axis, SpectrumKind)> = Axis::ALL
        .iter()
        .flat_map(|a| kinds.iter().map(move |k| (*a, *k)))
        .collect();
    let mut spectra: Vec<_> = jobs
        .par_iter()
        .filter_map(|(a, k)| {
            // High-rate batch tracks win over the uniform-grid series for gyro spectra.
            if matches!(k, SpectrumKind::GyroRaw | SpectrumKind::GyroFilt) {
                if let Some(track) = hr::track_for(log, *k) {
                    return hr::spectrum_hr(track, *a, &opts.spectrum, opts.range_s);
                }
            }
            spectrum::spectrum(log, *a, *k, &opts.spectrum, opts.range_s)
        })
        .collect();
    // ArduPilot: predicted post-filter spectrum from the logged filter parameters.
    if let domain::Tune::Ap(t) = &log.tune_at_log {
        let hover = log
            .meta
            .headers
            .get("ap.hover_thr")
            .and_then(|v| v.parse::<f32>().ok());
        let preds: Vec<_> = spectra
            .iter()
            .filter(|s| s.kind == SpectrumKind::GyroRaw)
            .map(|pre| {
                predicted::predict(pre, &predicted::chain_from_params(t, pre.fs_hz, hover, &[]))
            })
            .collect();
        spectra.extend(preds);
    }
    progress(0.6);

    let spectrograms: Vec<_> = Axis::ALL
        .par_iter()
        .filter_map(|&a| {
            if let Some(track) = hr::track_for(log, SpectrumKind::GyroRaw)
                .or_else(|| hr::track_for(log, SpectrumKind::GyroFilt))
            {
                return hr::spectrogram_hr(log, track, a, opts.spectrogram.max_hz);
            }
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
        .filter(|s| s.kind != SpectrumKind::Predicted)
        .flat_map(|s| peaks::find_peaks(s, &opts.peaks))
        .collect();

    let mut quality = quality::log_quality(log);
    for s in &steps {
        quality.step_segments_per_axis[s.axis.index()] = s.n_segments;
    }
    progress(1.0);

    let anomalies = anomaly::detect(log, quality.airborne_range_s, &opts.anomaly);
    // Betaflight CHIRP sweeps → closed-loop frequency response per axis
    let freq_resp: Vec<domain::FrequencyResponse> = if log
        .chirp
        .as_ref()
        .map(|c| !c.segments.is_empty())
        .unwrap_or(false)
    {
        Axis::ALL
            .par_iter()
            .filter_map(|&a| chirp::frequency_response(log, a, &opts.chirp))
            .collect()
    } else {
        Vec::new()
    };
    for fr in &freq_resp {
        let k = fr.axis.index();
        quality.chirp_sweeps_per_axis[k] = fr.n_sweeps;
        quality.chirp_windows_per_axis[k] = fr.n_windows;
        quality.chirp_coherence_per_axis[k] = if fr.metrics.coherence_mean.is_finite() {
            fr.metrics.coherence_mean
        } else {
            0.0
        };
    }
    AnalysisBundle {
        log: log.id.clone(),
        quality,
        steps,
        spectra,
        spectrograms,
        peaks,
        anomalies,
        freq_resp,
    }
}

/// Index range of `log.t` covering `range_s` (whole log if `None`).
pub(crate) fn range_idx(log: &FlightLog, range_s: Option<(f32, f32)>) -> (usize, usize) {
    match range_s {
        None => (0, log.len()),
        Some((t0, t1)) => dsp::decimate::range_indices(&log.t, t0, t1),
    }
}
