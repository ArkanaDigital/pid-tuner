//! Spectra from high-rate gyro tracks (ArduPilot IMU batch sampler).
//! Welch per batch, linear-power average across batches — the same
//! estimator ArduPilot's FilterReview uses on `ISBH/ISBD` data.

use crate::spectrum::{auto_nfft, SpectrumOpts};
use domain::{Axis, FlightLog, RawGyroTrack, Spectrogram, Spectrum, SpectrumKind};
use dsp::welch::{welch, WelchOpts};

fn track_kind(t: &RawGyroTrack) -> SpectrumKind {
    if t.post_filter {
        SpectrumKind::GyroFilt
    } else {
        SpectrumKind::GyroRaw
    }
}

/// Pick the track to use for `kind`: lowest instance, matching pre/post.
pub fn track_for(log: &FlightLog, kind: SpectrumKind) -> Option<&RawGyroTrack> {
    log.gyro_hr
        .iter()
        .filter(|t| track_kind(t) == kind)
        .min_by_key(|t| t.instance)
}

/// Averaged PSD over all batches (optionally restricted to `range_s`).
pub fn spectrum_hr(
    track: &RawGyroTrack,
    axis: Axis,
    opts: &SpectrumOpts,
    range_s: Option<(f32, f32)>,
) -> Option<Spectrum> {
    let k = axis.index();
    let batches: Vec<&domain::GyroBatch> = track
        .batches
        .iter()
        .filter(|b| {
            range_s
                .map(|(a, z)| b.t0_s >= a && b.t0_s <= z)
                .unwrap_or(true)
        })
        .filter(|b| b.xyz[k].len() >= 64)
        .collect();
    if batches.is_empty() {
        return None;
    }
    let min_len = batches.iter().map(|b| b.xyz[k].len()).min().unwrap();
    let nfft = if opts.nfft == 0 {
        auto_nfft(track.fs_hz, min_len)
    } else {
        opts.nfft.min(min_len)
    };
    let mut acc: Vec<f64> = Vec::new();
    let mut f_hz = Vec::new();
    let mut n = 0usize;
    for b in &batches {
        let psd = welch(
            &b.xyz[k],
            track.fs_hz,
            WelchOpts {
                nfft,
                overlap: opts.overlap,
                ..Default::default()
            },
        );
        if psd.n_windows == 0 {
            continue;
        }
        if acc.is_empty() {
            acc = vec![0.0; psd.psd_db.len()];
            f_hz = psd.f_hz;
        }
        for (a, d) in acc.iter_mut().zip(&psd.psd_db) {
            *a += 10f64.powf(*d as f64 / 10.0);
        }
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let psd_db: Vec<f32> = acc
        .iter()
        .map(|p| (10.0 * (p / n as f64).max(1e-30).log10()) as f32)
        .collect();
    let keep = if opts.max_hz > 0.0 {
        f_hz.iter().take_while(|f| **f <= opts.max_hz).count()
    } else {
        f_hz.len()
    };
    Some(Spectrum {
        axis,
        kind: track_kind(track),
        f_hz: f_hz[..keep].to_vec(),
        psd_db: psd_db[..keep].to_vec(),
        nfft,
        fs_hz: track.fs_hz,
    })
}

/// Throttle-vs-frequency map from batches: each batch is one spectrum
/// assigned to the throttle at its start time.
pub fn spectrogram_hr(
    log: &FlightLog,
    track: &RawGyroTrack,
    axis: Axis,
    max_hz: f32,
) -> Option<Spectrogram> {
    let k = axis.index();
    let batches: Vec<&domain::GyroBatch> = track
        .batches
        .iter()
        .filter(|b| b.xyz[k].len() >= 64)
        .collect();
    if batches.is_empty() {
        return None;
    }
    let min_len = batches.iter().map(|b| b.xyz[k].len()).min().unwrap();
    let nfft = auto_nfft(track.fs_hz, min_len).min(1024);
    let bins: Vec<f32> = (0..=100).map(|b| b as f32).collect();
    let mut f_all = Vec::new();
    let mut per_bin: Vec<(Vec<f64>, u32)> = vec![(Vec::new(), 0); bins.len()];
    for b in &batches {
        let psd = welch(
            &b.xyz[k],
            track.fs_hz,
            WelchOpts {
                nfft,
                overlap: 0.5,
                ..Default::default()
            },
        );
        if psd.n_windows == 0 {
            continue;
        }
        if f_all.is_empty() {
            f_all = psd
                .f_hz
                .iter()
                .copied()
                .take_while(|f| *f <= max_hz)
                .collect();
        }
        let ti = dsp::decimate::range_indices(&log.t, b.t0_s, b.t0_s)
            .0
            .min(log.throttle.len().saturating_sub(1));
        let thr = (log.throttle.get(ti).copied().unwrap_or(0.0) * 100.0)
            .round()
            .clamp(0.0, 100.0) as usize;
        let e = &mut per_bin[thr];
        if e.0.is_empty() {
            e.0 = vec![0.0; f_all.len()];
        }
        for (a, d) in e.0.iter_mut().zip(&psd.psd_db) {
            *a += 10f64.powf(*d as f64 / 10.0);
        }
        e.1 += 1;
    }
    let nf = f_all.len();
    let mut db = vec![-60f32; bins.len() * nf];
    let mut counts = vec![0u32; bins.len()];
    for (bi, (acc, n)) in per_bin.iter().enumerate() {
        counts[bi] = *n;
        if *n > 0 {
            for kf in 0..nf {
                db[bi * nf + kf] = (10.0 * (acc[kf] / *n as f64).max(1e-30).log10()) as f32;
            }
        }
    }
    Some(Spectrogram {
        axis,
        kind: track_kind(track),
        f_hz: f_all,
        throttle_bins: bins,
        db,
        counts,
    })
}
