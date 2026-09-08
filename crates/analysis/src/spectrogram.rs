//! Throttle-vs-frequency spectrogram, port of PIDtoolbox `PTthrSpec.m`:
//! 300 ms Hann segments, PSD in dB, grouped into 1 % throttle bins (±1 %).

use crate::range_idx;
use crate::spectrum::series;
use domain::{Axis, FlightLog, Spectrogram, SpectrumKind};
use dsp::fft::RealFft;
use dsp::window::hann;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrogramOpts {
    pub segment_s: f32,
    pub hop_s: f32,
    /// Half-width of the throttle bin window in percent (PIDtoolbox: 1).
    pub bin_halfwidth_pct: f32,
    pub max_hz: f32,
    /// Bins with fewer segments than this are left at the floor.
    pub min_count: u32,
}

impl Default for SpectrogramOpts {
    fn default() -> Self {
        Self {
            segment_s: 0.3,
            hop_s: 0.15,
            bin_halfwidth_pct: 1.0,
            max_hz: 1000.0,
            min_count: 1,
        }
    }
}

pub fn throttle_spectrogram(
    log: &FlightLog,
    axis: Axis,
    kind: SpectrumKind,
    opts: &SpectrogramOpts,
    range_s: Option<(f32, f32)>,
) -> Option<Spectrogram> {
    let s = series(log, axis, kind)?;
    let (i0, i1) = range_idx(log, range_s);
    let x = &s[i0.min(s.len())..i1.min(s.len())];
    let thr = &log.throttle[i0.min(log.throttle.len())..i1.min(log.throttle.len())];
    let fs = log.fs_hz;
    let seg = ((opts.segment_s * fs as f32) as usize).max(32) & !1usize;
    let hop = ((opts.hop_s * fs as f32) as usize).max(1);
    if x.len() < seg {
        return None;
    }
    let w = hann(seg);
    let mut fft = RealFft::new(seg);
    let nb = fft.out_len();
    let f_all = fft.freqs(fs);
    let nf = f_all
        .iter()
        .take_while(|f| **f <= opts.max_hz)
        .count()
        .max(1);

    // Per-segment PSD (linear) + mean throttle %.
    let mut seg_psd: Vec<Vec<f32>> = Vec::new();
    let mut seg_thr: Vec<f32> = Vec::new();
    let mut buf = vec![0f32; seg];
    let mut start = 0;
    while start + seg <= x.len() {
        let sl = &x[start..start + seg];
        let m = sl.iter().sum::<f32>() / seg as f32;
        for (b, (v, wi)) in buf.iter_mut().zip(sl.iter().zip(&w)) {
            *b = (v - m) * wi;
        }
        let spec = fft.forward(&mut buf);
        let psd: Vec<f32> = spec[..nf]
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let two = if k == 0 || k == nb - 1 { 1.0 } else { 2.0 };
                two * c.norm_sqr() / (fs as f32 * seg as f32)
            })
            .collect();
        seg_psd.push(psd);
        let t = &thr[start..(start + seg).min(thr.len())];
        seg_thr.push(100.0 * t.iter().sum::<f32>() / t.len().max(1) as f32);
        start += hop;
    }
    if seg_psd.is_empty() {
        return None;
    }

    let bins: Vec<f32> = (0..=100).map(|b| b as f32).collect();
    let mut db = vec![-60f32; bins.len() * nf];
    let mut counts = vec![0u32; bins.len()];
    let hw = opts.bin_halfwidth_pct;
    for (bi, &b) in bins.iter().enumerate() {
        let mut acc = vec![0f32; nf];
        let mut n = 0u32;
        for (p, &t) in seg_psd.iter().zip(&seg_thr) {
            if t > b - hw && t <= b + hw {
                for k in 0..nf {
                    acc[k] += p[k];
                }
                n += 1;
            }
        }
        counts[bi] = n;
        if n >= opts.min_count.max(1) {
            for k in 0..nf {
                db[bi * nf + k] = 10.0 * (acc[k] / n as f32).max(1e-30).log10();
            }
        }
    }
    Some(Spectrogram {
        axis,
        kind,
        f_hz: f_all[..nf].to_vec(),
        throttle_bins: bins,
        db,
        counts,
    })
}
