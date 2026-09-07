//! Noise peak detection on a PSD: prominence above a running-median floor.

use domain::{NoiseBand, NoisePeak, Spectrum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakOpts {
    /// Ignore everything below this (craft motion / propwash).
    pub min_hz: f32,
    /// Width of the running-median floor window in Hz.
    pub floor_window_hz: f32,
    /// Minimum prominence above the floor (dB).
    pub min_prominence_db: f32,
    /// Minimum spacing between reported peaks (Hz).
    pub min_spacing_hz: f32,
    pub max_peaks: usize,
}

impl Default for PeakOpts {
    fn default() -> Self {
        Self { min_hz: 20.0, floor_window_hz: 60.0, min_prominence_db: 8.0, min_spacing_hz: 15.0, max_peaks: 8 }
    }
}

pub fn band_for(f: f32) -> NoiseBand {
    if f < 100.0 {
        NoiseBand::Control
    } else if f < 250.0 {
        NoiseBand::Frame
    } else {
        NoiseBand::Motor
    }
}

fn running_median(x: &[f32], half: usize) -> Vec<f32> {
    let n = x.len();
    let mut out = vec![0f32; n];
    let mut buf = Vec::with_capacity(2 * half + 1);
    for i in 0..n {
        let a = i.saturating_sub(half);
        let b = (i + half + 1).min(n);
        buf.clear();
        buf.extend_from_slice(&x[a..b]);
        buf.sort_by(|p, q| p.partial_cmp(q).unwrap());
        out[i] = buf[buf.len() / 2];
    }
    out
}

pub fn find_peaks(s: &Spectrum, opts: &PeakOpts) -> Vec<NoisePeak> {
    let n = s.f_hz.len();
    if n < 8 {
        return vec![];
    }
    let df = (s.fs_hz / s.nfft as f64) as f32;
    let half = ((opts.floor_window_hz / df) / 2.0).max(1.0) as usize;
    let floor = running_median(&s.psd_db, half);
    let prom: Vec<f32> = s.psd_db.iter().zip(&floor).map(|(p, f)| p - f).collect();

    let mut cands: Vec<(usize, f32)> = (1..n - 1)
        .filter(|&i| s.f_hz[i] >= opts.min_hz)
        .filter(|&i| prom[i] >= opts.min_prominence_db)
        .filter(|&i| s.psd_db[i] >= s.psd_db[i - 1] && s.psd_db[i] > s.psd_db[i + 1])
        .map(|i| (i, prom[i]))
        .collect();
    cands.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let mut out: Vec<NoisePeak> = Vec::new();
    for (i, p) in cands {
        if out.len() >= opts.max_peaks {
            break;
        }
        let f = s.f_hz[i];
        if out.iter().any(|q| (q.f_hz - f).abs() < opts.min_spacing_hz) {
            continue;
        }
        out.push(NoisePeak { axis: s.axis, kind: s.kind, f_hz: f, psd_db: s.psd_db[i], prominence_db: p, band: band_for(f) });
    }
    out.sort_by(|a, b| a.f_hz.partial_cmp(&b.f_hz).unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Axis, SpectrumKind};

    #[test]
    fn finds_two_synthetic_peaks() {
        let f_hz: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let psd: Vec<f32> = f_hz
            .iter()
            .map(|f| {
                let mut v = -40.0 - f / 100.0;
                if (f - 200.0).abs() < 4.0 {
                    v += 25.0 * (1.0 - (f - 200.0).abs() / 4.0);
                }
                if (f - 600.0).abs() < 10.0 {
                    v += 15.0 * (1.0 - (f - 600.0).abs() / 10.0);
                }
                v
            })
            .collect();
        let s = Spectrum { axis: Axis::Roll, kind: SpectrumKind::GyroRaw, f_hz, psd_db: psd, nfft: 2000, fs_hz: 2000.0 };
        let p = find_peaks(&s, &PeakOpts::default());
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(p[0].f_hz as i32, 200);
        assert_eq!(p[0].band, NoiseBand::Frame);
        assert_eq!(p[1].f_hz as i32, 600);
        assert_eq!(p[1].band, NoiseBand::Motor);
    }
}
