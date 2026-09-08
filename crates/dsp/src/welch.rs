//! Power spectral density estimation.
//!
//! Two flavours:
//! - [`welch`]: averaged, Hann-windowed, overlapped PSD with window energy correction,
//!   scaled like ArduPilot WebTools `fft.js` (`PSD (dB/Hz)`).
//! - [`periodogram_pidtoolbox`]: single Hann window over the whole segment without
//!   correction, exactly PIDtoolbox `PTSpec2d.m` ("Full Spectrum").
//!
//! Both return one-sided spectra in dB.

use crate::fft::RealFft;
use crate::window::{hann, window_correction};

#[derive(Debug, Clone)]
pub struct Psd {
    pub f_hz: Vec<f32>,
    pub psd_db: Vec<f32>,
    pub nfft: usize,
    pub fs_hz: f64,
    pub n_windows: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct WelchOpts {
    pub nfft: usize,
    /// 0.0..1.0, fraction of `nfft` shared by consecutive windows.
    pub overlap: f32,
    /// Apply Hann energy correction (WebTools) — off reproduces PIDtoolbox scaling.
    pub window_correction: bool,
    /// Remove the mean of each window before the FFT.
    pub detrend: bool,
}

impl Default for WelchOpts {
    fn default() -> Self {
        Self {
            nfft: 1024,
            overlap: 0.5,
            window_correction: true,
            detrend: true,
        }
    }
}

/// One-sided PSD in dB/Hz using linear power averaging across windows.
pub fn welch(x: &[f32], fs: f64, opts: WelchOpts) -> Psd {
    let nfft = opts.nfft.max(16);
    let hop = ((nfft as f32) * (1.0 - opts.overlap)).max(1.0) as usize;
    let w = hann(nfft);
    let corr = if opts.window_correction {
        window_correction(&w).energy
    } else {
        1.0
    };
    let mut fft = RealFft::new(nfft);
    let nb = fft.out_len();
    let mut acc = vec![0f64; nb];
    let mut n_windows = 0usize;
    let mut buf = vec![0f32; nfft];

    if x.len() >= nfft {
        let mut start = 0usize;
        while start + nfft <= x.len() {
            let seg = &x[start..start + nfft];
            let m = if opts.detrend {
                seg.iter().sum::<f32>() / nfft as f32
            } else {
                0.0
            };
            for (b, (s, wi)) in buf.iter_mut().zip(seg.iter().zip(&w)) {
                *b = (s - m) * wi;
            }
            let spec = fft.forward(&mut buf);
            for (k, c) in spec.iter().enumerate() {
                acc[k] += c.norm_sqr() as f64;
            }
            n_windows += 1;
            start += hop;
        }
    }

    // Scale: PSD = 2|X|²·corr² / (N·fs), DC and Nyquist not doubled.
    let n = nfft as f64;
    let corr2 = (corr as f64) * (corr as f64);
    let psd_db: Vec<f32> = acc
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let p = if n_windows > 0 {
                p / n_windows as f64
            } else {
                0.0
            };
            let two = if k == 0 || k == nb - 1 { 1.0 } else { 2.0 };
            let v = two * p * corr2 / (n * fs);
            (10.0 * v.max(1e-30).log10()) as f32
        })
        .collect();

    Psd {
        f_hz: fft.freqs(fs),
        psd_db,
        nfft,
        fs_hz: fs,
        n_windows,
    }
}

/// PIDtoolbox `PTSpec2d.m`: one Hann window over the entire signal, no averaging,
/// `psd = 10·log10(2|Y|²/(Fs·N))`. `nfft` is `x.len()` rounded to even.
pub fn periodogram_pidtoolbox(x: &[f32], fs: f64) -> Psd {
    let n = x.len() & !1usize;
    if n < 16 {
        return Psd {
            f_hz: vec![],
            psd_db: vec![],
            nfft: n,
            fs_hz: fs,
            n_windows: 0,
        };
    }
    let w = hann(n);
    let mut buf: Vec<f32> = x[..n].iter().zip(&w).map(|(a, b)| a * b).collect();
    let mut fft = RealFft::new(n);
    let spec = fft.forward(&mut buf);
    let nb = spec.len();
    let psd_db = spec
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let two = if k == 0 || k == nb - 1 { 1.0 } else { 2.0 };
            let v = two * c.norm_sqr() as f64 / (fs * n as f64);
            (10.0 * v.max(1e-30).log10()) as f32
        })
        .collect();
    Psd {
        f_hz: fft.freqs(fs),
        psd_db,
        nfft: n,
        fs_hz: fs,
        n_windows: 1,
    }
}

/// Index of the bin closest to `f`.
pub fn bin_for(psd: &Psd, f: f32) -> usize {
    let df = psd.fs_hz as f32 / psd.nfft as f32;
    ((f / df).round() as usize).min(psd.f_hz.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn sine(f: f32, fs: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * f * i as f32 / fs).sin())
            .collect()
    }

    #[test]
    fn welch_sine_peak_location_and_level() {
        // amplitude 1 sine → power 0.5 W concentrated in one bin of width fs/N.
        let fs = 2000.0;
        let x = sine(250.0, fs, 32768, 1.0);
        let psd = welch(
            &x,
            fs as f64,
            WelchOpts {
                nfft: 2048,
                ..Default::default()
            },
        );
        let k = bin_for(&psd, 250.0);
        let (kmax, _) = psd
            .psd_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert_eq!(k, kmax);
        // Expected PSD ≈ 10log10(0.5 / (fs/N)) ; Hann spreads over ~1.5 bins so allow ~2 dB.
        let expected = 10.0 * (0.5f32 / (fs / 2048.0)).log10();
        assert_relative_eq!(psd.psd_db[k], expected, epsilon = 2.0);
        assert!(psd.n_windows > 20);
    }

    #[test]
    fn pidtoolbox_periodogram_peak() {
        let fs = 2000.0;
        let x = sine(600.0, fs, 4000, 2.0);
        let psd = periodogram_pidtoolbox(&x, fs as f64);
        let (kmax, _) = psd
            .psd_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert_relative_eq!(psd.f_hz[kmax], 600.0, epsilon = 1.0);
    }
}
