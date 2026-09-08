//! Cross-spectral estimation for closed-loop system identification.
//!
//! Port of Betaflight Configurator `src/js/blackbox/spectral_analysis.js`
//! (GPL-3.0-or-later, `welchTransferFunction`, `computeOpenLoop`,
//! `computeSensitivity`, `computeStepResponse`), with one deliberate change:
//! each window is mean-detrended before the FFT (the original is not, which
//! biases the lowest bins).
//!
//! Everything downstream is a ratio, so the accumulated spectra are raw sums
//! (no `1/N`, no window-power correction), exactly like the original.

use crate::fft::{ComplexFft, RealFft};
use crate::window::hann;
use num_complex::{Complex32, Complex64};

#[derive(Debug, Clone, Copy)]
pub struct CsdOpts {
    pub nfft: usize,
    /// Fraction of `nfft` shared by consecutive windows (Configurator: 0.5).
    pub overlap: f32,
    /// Remove the window mean before the FFT.
    pub detrend: bool,
}

impl Default for CsdOpts {
    fn default() -> Self {
        Self {
            nfft: 1024,
            overlap: 0.5,
            detrend: true,
        }
    }
}

/// Accumulated auto/cross spectra (one-sided, raw sums over windows).
#[derive(Debug, Clone)]
pub struct CrossSpectrum {
    pub f_hz: Vec<f32>,
    pub sxx: Vec<f64>,
    pub syy: Vec<f64>,
    /// `Σ conj(U)·Y`
    pub sxy: Vec<Complex64>,
    pub n_windows: usize,
    pub nfft: usize,
    pub fs_hz: f64,
}

/// Configurator `chooseSegmentSize`: 256, doubled while `< 0.5·fs`, capped at 4096.
pub fn segment_size_for(fs: f64) -> usize {
    let mut n = 256usize;
    while (n as f64) < fs * 0.5 {
        n <<= 1;
    }
    n.min(4096)
}

impl CrossSpectrum {
    pub fn empty(nfft: usize, fs: f64) -> Self {
        let nb = nfft / 2 + 1;
        Self {
            f_hz: (0..nb)
                .map(|k| (k as f64 * fs / nfft as f64) as f32)
                .collect(),
            sxx: vec![0.0; nb],
            syy: vec![0.0; nb],
            sxy: vec![Complex64::default(); nb],
            n_windows: 0,
            nfft,
            fs_hz: fs,
        }
    }

    /// Add another estimate with the same `nfft`/`fs` (windows from a further sweep).
    pub fn accumulate(&mut self, other: &CrossSpectrum) {
        assert_eq!(self.nfft, other.nfft);
        for k in 0..self.sxx.len() {
            self.sxx[k] += other.sxx[k];
            self.syy[k] += other.syy[k];
            self.sxy[k] += other.sxy[k];
        }
        self.n_windows += other.n_windows;
    }

    /// H₁ estimate `Sxy / Sxx`; bins with no input power become NaN.
    pub fn transfer(&self) -> Vec<Complex32> {
        self.sxx
            .iter()
            .zip(&self.sxy)
            .map(|(&sxx, sxy)| {
                if sxx < 1e-20 {
                    Complex32::new(f32::NAN, f32::NAN)
                } else {
                    Complex32::new((sxy.re / sxx) as f32, (sxy.im / sxx) as f32)
                }
            })
            .collect()
    }

    /// Magnitude-squared coherence `|Sxy|² / (Sxx·Syy)`, 0 where undefined.
    pub fn coherence(&self) -> Vec<f32> {
        (0..self.sxx.len())
            .map(|k| {
                let d = self.sxx[k] * self.syy[k];
                if d > 1e-30 {
                    (self.sxy[k].norm_sqr() / d).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                }
            })
            .collect()
    }
}

/// Welch cross-spectrum of input `u` and output `y` (same length, same rate).
pub fn cross_welch(u: &[f32], y: &[f32], fs: f64, o: CsdOpts) -> CrossSpectrum {
    let nfft = o.nfft.max(16);
    let mut out = CrossSpectrum::empty(nfft, fs);
    let n = u.len().min(y.len());
    if n < nfft {
        return out;
    }
    let hop = ((nfft as f32) * (1.0 - o.overlap)).round().max(1.0) as usize;
    let w = hann(nfft);
    let mut fft = RealFft::new(nfft);
    let mut bu = vec![0f32; nfft];
    let mut by = vec![0f32; nfft];
    let mut start = 0usize;
    while start + nfft <= n {
        let su = &u[start..start + nfft];
        let sy = &y[start..start + nfft];
        let (mu, my) = if o.detrend {
            (
                su.iter().sum::<f32>() / nfft as f32,
                sy.iter().sum::<f32>() / nfft as f32,
            )
        } else {
            (0.0, 0.0)
        };
        for k in 0..nfft {
            bu[k] = (su[k] - mu) * w[k];
            by[k] = (sy[k] - my) * w[k];
        }
        let fu = fft.forward(&mut bu);
        let fy = fft.forward(&mut by);
        for k in 0..out.sxx.len() {
            let (a, b) = (fu[k], fy[k]);
            out.sxx[k] += a.norm_sqr() as f64;
            out.syy[k] += b.norm_sqr() as f64;
            // conj(U)·Y
            let c = Complex64::new(
                (a.re * b.re + a.im * b.im) as f64,
                (a.re * b.im - a.im * b.re) as f64,
            );
            out.sxy[k] += c;
        }
        out.n_windows += 1;
        start += hop;
    }
    out
}

/// Unwrap a phase series in degrees in place (±360° steps on jumps > 180°).
pub fn unwrap_deg(phase: &mut [f32]) {
    let mut offset = 0.0f32;
    let mut prev: Option<f32> = None;
    for p in phase.iter_mut() {
        if !p.is_finite() {
            continue;
        }
        let raw = *p;
        if let Some(pr) = prev {
            let mut d = raw + offset - pr;
            while d > 180.0 {
                offset -= 360.0;
                d -= 360.0;
            }
            while d < -180.0 {
                offset += 360.0;
                d += 360.0;
            }
        }
        *p = raw + offset;
        prev = Some(*p);
    }
}

/// Open loop from the closed loop: `L = H / (1 − H)`.
pub fn open_loop(h: &[Complex32]) -> Vec<Complex32> {
    h.iter()
        .map(|&h| {
            if !h.re.is_finite() || !h.im.is_finite() {
                return Complex32::new(f32::NAN, f32::NAN);
            }
            let d = Complex32::new(1.0 - h.re, -h.im);
            if d.norm_sqr() < 1e-20 {
                Complex32::new(f32::INFINITY, 0.0)
            } else {
                h / d
            }
        })
        .collect()
}

/// Sensitivity `S = 1 − H` (= 1/(1+L)).
pub fn sensitivity(h: &[Complex32]) -> Vec<Complex32> {
    h.iter()
        .map(|&h| Complex32::new(1.0 - h.re, -h.im))
        .collect()
}

/// Predicted sensitivity peak `max 1/|1 + g·L|` over finite bins.
pub fn predicted_sens_peak(l: &[Complex32], g: f32) -> f32 {
    l.iter()
        .filter(|c| c.re.is_finite() && c.im.is_finite())
        .map(|c| {
            let d = Complex32::new(1.0 + g * c.re, g * c.im);
            1.0 / d.norm().max(1e-9)
        })
        .fold(0.0f32, f32::max)
}

/// Step response from H: Hermitian-mirrored inverse FFT → cumulative sum,
/// normalised by |H(0)|, truncated to `len_ms`. Returns `(t_ms, step)`.
pub fn step_from_h(h: &[Complex32], nfft: usize, fs: f64, len_ms: f32) -> (Vec<f32>, Vec<f32>) {
    let nb = h.len();
    if nb == 0 || nfft < 2 * (nb - 1) {
        return (vec![], vec![]);
    }
    let mut spec = vec![Complex32::default(); nfft];
    for k in 0..nb {
        let v = h[k];
        spec[k] = if v.re.is_finite() && v.im.is_finite() {
            v
        } else {
            Complex32::default()
        };
    }
    for k in nb..nfft {
        let mk = nfft - k;
        spec[k] = spec[mk].conj();
    }
    let mut fft = ComplexFft::new(nfft);
    let imp = fft.inverse_real(spec);
    let n = ((len_ms / 1000.0) * fs as f32).round() as usize;
    let n = n.min(nfft / 2).max(1);
    let dc = spec_dc(h);
    let mut step = Vec::with_capacity(n);
    let mut acc = 0.0f32;
    for v in imp.iter().take(n) {
        acc += v;
        step.push(if dc > 1e-10 { acc / dc } else { acc });
    }
    let t: Vec<f32> = (0..n).map(|i| i as f32 * 1000.0 / fs as f32).collect();
    (t, step)
}

fn spec_dc(h: &[Complex32]) -> f32 {
    // |H| at the lowest finite bin (DC itself may be NaN after detrending).
    h.iter()
        .find(|c| c.re.is_finite() && c.im.is_finite())
        .map(|c| c.norm())
        .unwrap_or(0.0)
}

/// First crossing of `level` by `y` (going down or up, whichever first) between
/// consecutive valid bins, linearly interpolated in `f`. NaN when none.
pub fn interp_crossing(f: &[f32], y: &[f32], level: f32, valid: &[bool], downward: bool) -> f32 {
    for k in 1..f.len().min(y.len()) {
        if !valid[k] || !valid[k - 1] || !y[k].is_finite() || !y[k - 1].is_finite() {
            continue;
        }
        let hit = if downward {
            y[k] <= level && y[k - 1] > level
        } else {
            y[k] >= level && y[k - 1] < level
        };
        if hit {
            let frac = (level - y[k - 1]) / (y[k] - y[k - 1]);
            return f[k - 1] + frac * (f[k] - f[k - 1]);
        }
    }
    f32::NAN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random noise (xorshift), zero mean-ish.
    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
            })
            .collect()
    }

    fn fir(x: &[f32], taps: &[f32]) -> Vec<f32> {
        (0..x.len())
            .map(|i| {
                taps.iter()
                    .enumerate()
                    .map(|(k, t)| if i >= k { t * x[i - k] } else { 0.0 })
                    .sum()
            })
            .collect()
    }

    fn fir_response(taps: &[f32], f: f32, fs: f32) -> Complex32 {
        taps.iter()
            .enumerate()
            .fold(Complex32::default(), |acc, (k, t)| {
                acc + Complex32::from_polar(*t, -std::f32::consts::TAU * f * k as f32 / fs)
            })
    }

    #[test]
    fn segment_size_table() {
        assert_eq!(segment_size_for(1000.0), 512);
        assert_eq!(segment_size_for(2000.0), 1024);
        assert_eq!(segment_size_for(4000.0), 2048);
        assert_eq!(segment_size_for(8000.0), 4096);
        assert_eq!(segment_size_for(16000.0), 4096);
    }

    #[test]
    fn fir_plant_is_recovered_with_high_coherence() {
        let fs = 2000.0f32;
        let taps = [0.5f32, 0.3, 0.15, 0.05];
        let u = noise(200_000, 0x9E3779B97F4A7C15);
        let y = fir(&u, &taps);
        let cs = cross_welch(
            &u,
            &y,
            fs as f64,
            CsdOpts {
                nfft: 512,
                overlap: 0.5,
                detrend: true,
            },
        );
        let h = cs.transfer();
        let coh = cs.coherence();
        for (k, f) in cs.f_hz.iter().enumerate() {
            if *f < 2.0 || *f > 0.4 * fs {
                continue;
            }
            let want = fir_response(&taps, *f, fs);
            let mag_err = 20.0 * (h[k].norm() / want.norm()).log10();
            let ph_err = (h[k].arg() - want.arg()).to_degrees();
            let ph_err = ((ph_err + 180.0).rem_euclid(360.0)) - 180.0;
            assert!(mag_err.abs() < 0.5, "f={f} mag err {mag_err} dB");
            assert!(ph_err.abs() < 3.0, "f={f} phase err {ph_err} deg");
            assert!(coh[k] > 0.97, "f={f} coherence {}", coh[k]);
        }
        assert!(cs.n_windows > 700);
    }

    #[test]
    fn noiseless_pair_has_unit_coherence_and_noise_lowers_it() {
        let fs = 2000.0;
        let n = 60_000;
        let u: Vec<f32> = (0..n)
            .map(|i| {
                (std::f32::consts::TAU * 37.0 * i as f32 / fs).sin()
                    + 0.3 * (std::f32::consts::TAU * 91.0 * i as f32 / fs).sin()
            })
            .collect();
        let y: Vec<f32> = u.iter().map(|v| 0.8 * v).collect();
        let cs = cross_welch(&u, &y, fs as f64, CsdOpts::default());
        let coh = cs.coherence();
        let k37 = (37.0 / (fs / 1024.0)).round() as usize;
        assert!((coh[k37] - 1.0).abs() < 1e-3, "{}", coh[k37]);
        let yn: Vec<f32> = y.iter().zip(noise(n, 7)).map(|(a, b)| a + b).collect();
        let cs2 = cross_welch(&u, &yn, fs as f64, CsdOpts::default());
        let coh2 = cs2.coherence();
        let k500 = (500.0 / (fs / 1024.0)).round() as usize;
        assert!(coh2[k500] < 0.5, "{}", coh2[k500]);
        assert!(coh2[k37] > 0.9, "{}", coh2[k37]);
    }

    #[test]
    fn detrend_protects_low_bins() {
        let fs = 2000.0f32;
        let u = noise(100_000, 42);
        let y: Vec<f32> = u.iter().map(|v| 0.7 * v).collect();
        let y_off: Vec<f32> = y.iter().map(|v| v + 50.0).collect();
        let a = cross_welch(
            &u,
            &y_off,
            fs as f64,
            CsdOpts {
                nfft: 1024,
                overlap: 0.5,
                detrend: true,
            },
        );
        let b = cross_welch(
            &u,
            &y_off,
            fs as f64,
            CsdOpts {
                nfft: 1024,
                overlap: 0.5,
                detrend: false,
            },
        );
        let ha = a.transfer();
        let hb = b.transfer();
        // with detrend the whole band reads 0.7
        for k in 1..ha.len() {
            if a.f_hz[k] >= 2.0 && a.f_hz[k] <= 800.0 {
                assert!(
                    (20.0 * (ha[k].norm() / 0.7).log10()).abs() < 0.1,
                    "f={} {}",
                    a.f_hz[k],
                    ha[k].norm()
                );
            }
        }
        // without detrend the DC/first bins are corrupted by the offset
        assert!(
            (hb[0].norm() - 0.7).abs() > 0.5 || (hb[1].norm() - 0.7).abs() > 0.05,
            "dc={} b1={}",
            hb[0].norm(),
            hb[1].norm()
        );
    }

    #[test]
    fn step_from_h_unit_and_first_order() {
        let fs = 2000.0;
        let nfft = 1024;
        let nb = nfft / 2 + 1;
        let ones: Vec<Complex32> = vec![Complex32::new(1.0, 0.0); nb];
        let (t, s) = step_from_h(&ones, nfft, fs, 100.0);
        assert_eq!(t.len(), 200);
        assert!(
            (s[0] - 1.0).abs() < 1e-3 && (s[100] - 1.0).abs() < 1e-3,
            "{:?}",
            &s[..3]
        );
        // first order H = 1/(1 + j f/fc), fc = 20 Hz → τ = 7.96 ms → 63 % at τ
        let fc = 20.0f32;
        let h: Vec<Complex32> = (0..nb)
            .map(|k| {
                let f = k as f32 * fs as f32 / nfft as f32;
                Complex32::new(1.0, 0.0) / Complex32::new(1.0, f / fc)
            })
            .collect();
        let (t, s) = step_from_h(&h, nfft, fs, 100.0);
        let tau_ms = 1000.0 / (std::f32::consts::TAU * fc);
        let k = t.iter().position(|x| *x >= tau_ms).unwrap();
        assert!((s[k] - 0.632).abs() < 0.05, "{} at {} ms", s[k], t[k]);
    }

    #[test]
    fn unwrap_handles_720_degree_ramp() {
        let true_phase: Vec<f32> = (0..200).map(|i| -3.6 * i as f32).collect(); // 0 → −716.4
        let mut wrapped: Vec<f32> = true_phase
            .iter()
            .map(|p| ((p + 180.0).rem_euclid(360.0)) - 180.0)
            .collect();
        unwrap_deg(&mut wrapped);
        for (a, b) in wrapped.iter().zip(&true_phase) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn open_loop_inverts_closed_loop() {
        // L known → H = L/(1+L) → open_loop(H) == L
        let l = Complex32::new(0.4, -1.3);
        let h = l / (Complex32::new(1.0, 0.0) + l);
        let back = open_loop(&[h])[0];
        assert!((back - l).norm() < 1e-5);
        assert!(open_loop(&[Complex32::new(f32::NAN, 0.0)])[0].re.is_nan());
    }

    #[test]
    fn interp_crossing_linear() {
        let f = [0.0, 10.0, 20.0, 30.0];
        let y = [0.0, -1.0, -5.0, -9.0];
        let v = [true; 4];
        let x = interp_crossing(&f, &y, -3.0, &v, true);
        assert!((x - 15.0).abs() < 1e-5, "{x}");
        assert!(interp_crossing(&f, &y, 5.0, &v, false).is_nan());
    }
}
