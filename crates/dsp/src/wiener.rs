//! Wiener-deconvolution step response estimation.
//!
//! Given an input `u` (setpoint) and output `y` (gyro), estimate the impulse
//! response `h` such that `y ≈ u * h`, then integrate to a step response.
//!
//! Two regularizations, selectable via [`Regularization`]:
//! - `Constant(1e-4)`: PIDtoolbox `PTstepcalc.m`
//!   `G = fft(y)/N; H = fft(u)/N; imp = real(ifft((G.*conj(H)) ./ (H.*conj(H) + 0.0001)))`
//!   The constant is relative to spectra **normalised by 1/N**, i.e. it acts on
//!   per-bin amplitude squared in signal units (deg/s)². Internally we keep the
//!   unnormalised FFT and scale the constant by N² instead.
//! - `Gaussian { cutoff_hz: 25 }`: PID-Analyzer `wiener_deconvolution` /
//!   ArduPilot PIDReview `redraw_step()`: a smoothed frequency mask that is
//!   ~0.1 below the cutoff and huge above it.

use crate::fft::ComplexFft;
use num_complex::Complex32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Regularization {
    Constant(f32),
    Gaussian { cutoff_hz: f32 },
}

/// Deconvolve `y` by `u` and return the **impulse** response (length `n`, where
/// `n` is the padded FFT length). Both inputs must have equal length.
pub fn impulse_response(
    u: &[f32],
    y: &[f32],
    fs: f64,
    pad_to: usize,
    reg: Regularization,
) -> Vec<f32> {
    assert_eq!(u.len(), y.len());
    let n = pad_to.max(u.len());
    let mut fft = ComplexFft::new(n);
    let h = fft.forward_real(u);
    let g = fft.forward_real(y);

    let regv: Vec<f32> = match reg {
        Regularization::Constant(c) => vec![c * (n as f32) * (n as f32); n],
        Regularization::Gaussian { cutoff_hz } => gaussian_reg(&fft.abs_freqs(fs), cutoff_hz),
    };

    let spec: Vec<Complex32> = h
        .iter()
        .zip(&g)
        .zip(&regv)
        .map(|((hk, gk), r)| {
            let hcon = hk.conj();
            (gk * hcon) / (hk * hcon + Complex32::new(*r, 0.0))
        })
        .collect();

    fft.inverse_real(spec)
}

/// Port of PID-Analyzer's frequency mask.
///
/// `sn = 10·(1 − smooth(mask(f ≥ cutoff)) + 1e-9)`, regularization = `1/sn`.
/// Result: ≈0.1 in the passband, ≈1e8 far above the cutoff.
fn gaussian_reg(abs_freqs: &[f32], cutoff_hz: f32) -> Vec<f32> {
    let n = abs_freqs.len();
    // mask: 0 below cutoff, 1 above.
    let mask: Vec<f32> = abs_freqs
        .iter()
        .map(|f| if *f >= cutoff_hz { 1.0 } else { 0.0 })
        .collect();
    let len_lpf = mask.iter().filter(|m| **m == 0.0).count().max(1);
    let sigma = len_lpf as f32 / 6.0;
    let smoothed = gaussian_filter1d(&mask, sigma);
    // renormalize to 0..1 (PID-Analyzer `to_mask`)
    let (mn, mx) = smoothed
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let span = (mx - mn).max(1e-12);
    (0..n)
        .map(|i| {
            let s = (smoothed[i] - mn) / span;
            let sn = 10.0 * (1.0 - s + 1e-9);
            1.0 / sn
        })
        .collect()
}

/// Circular Gaussian smoothing (matches scipy's default `mode='reflect'` closely
/// enough for a symmetric spectrum; the mask is symmetric so wrap == reflect).
fn gaussian_filter1d(x: &[f32], sigma: f32) -> Vec<f32> {
    let n = x.len();
    if sigma <= 0.0 || n == 0 {
        return x.to_vec();
    }
    let radius = (4.0 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|k| (-(k as f32).powi(2) / (2.0 * sigma * sigma)).exp())
        .collect();
    let ksum: f32 = kernel.iter().sum();
    let mut out = vec![0f32; n];
    for i in 0..n {
        let mut acc = 0.0;
        for (j, kv) in kernel.iter().enumerate() {
            let off = j as isize - radius;
            let idx = (i as isize + off).rem_euclid(n as isize) as usize;
            acc += x[idx] * kv;
        }
        out[i] = acc / ksum;
    }
    out
}

/// Cumulative sum: impulse → step response.
pub fn cumsum(x: &[f32]) -> Vec<f32> {
    let mut acc = 0.0;
    x.iter()
        .map(|v| {
            acc += v;
            acc
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::hann;
    use approx::assert_relative_eq;

    fn lcg(seed: &mut u64) -> f32 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }

    /// Stick-like broadband input: random levels held 100–300 ms, 4 ms smoothing.
    fn stick_input(total: usize, fs: f32, seed: u64) -> Vec<f32> {
        let mut seed = seed;
        let mut u = vec![0f32; total];
        let mut i = 0;
        while i < total {
            let hold = (0.1 * fs + 0.2 * fs * (lcg(&mut seed) * 0.5 + 0.5)) as usize;
            let level = 500.0 * lcg(&mut seed);
            for v in u.iter_mut().skip(i).take(hold) {
                *v = level;
            }
            i += hold.max(1);
        }
        let sa = (-1.0 / (0.004 * fs)).exp();
        for i in 1..total {
            u[i] = sa * u[i - 1] + (1.0 - sa) * u[i];
        }
        u
    }

    fn first_order(u: &[f32], fs: f32, tau: f32) -> Vec<f32> {
        let a = (-1.0 / (tau * fs)).exp();
        let mut y = vec![0f32; u.len()];
        for i in 1..u.len() {
            y[i] = a * y[i - 1] + (1.0 - a) * u[i];
        }
        y
    }

    /// Average Hann-windowed segment estimates, like the analysis crate does.
    fn averaged_step(u: &[f32], y: &[f32], fs: f32, n: usize, reg: Regularization) -> Vec<f32> {
        let w = hann(n);
        let mut acc = vec![0f32; n];
        let mut cnt = 0;
        let mut s0 = n / 2;
        while s0 + n <= u.len() {
            let us: Vec<f32> = u[s0..s0 + n].iter().zip(&w).map(|(a, b)| a * b).collect();
            let ys: Vec<f32> = y[s0..s0 + n].iter().zip(&w).map(|(a, b)| a * b).collect();
            let step = cumsum(&impulse_response(&us, &ys, fs as f64, n + 100, reg));
            for k in 0..n {
                acc[k] += step[k];
            }
            cnt += 1;
            s0 += n / 8;
        }
        acc.iter().map(|v| v / cnt as f32).collect()
    }

    /// A delayed identity plant (causal, like any real plant) must give a unit
    /// step after the delay. (A pure identity is zero-phase, so half of the
    /// band-limited impulse lands at negative time — not a meaningful test.)
    #[test]
    fn delayed_identity_gives_unit_step_constant_reg() {
        let fs = 2000.0f32;
        let u = stick_input(40000, fs, 3);
        let delay = 10usize;
        let mut y = vec![0f32; u.len()];
        y[delay..].copy_from_slice(&u[..u.len() - delay]);
        let step = averaged_step(&u, &y, fs, 4000, Regularization::Constant(1e-4));
        for k in [40usize, 100, 500, 999] {
            assert_relative_eq!(step[k], 1.0, epsilon = 3e-2);
        }
        assert!(step[5] < 0.3);
    }

    /// Gaussian regularization band-limits the estimate to ~25 Hz. A pure
    /// identity is zero-phase (half the impulse lands at negative time), so use
    /// a 5 ms delayed identity, which is causal like any real plant.
    #[test]
    fn delayed_identity_gives_unit_step_gaussian_reg() {
        let fs = 2000.0f32;
        let u = stick_input(40000, fs, 7);
        let delay = 10usize;
        let mut y = vec![0f32; u.len()];
        y[delay..].copy_from_slice(&u[..u.len() - delay]);
        let step = averaged_step(&u, &y, fs, 4000, Regularization::Gaussian { cutoff_hz: 25.0 });
        for k in [200usize, 400, 800] {
            assert_relative_eq!(step[k], 1.0, epsilon = 6e-2);
        }
        assert!(step[5] < 0.3, "before the delay the response must be near zero: {}", step[5]);
        assert!(step[60] > 0.8);
    }

    /// A first-order lag plant yields the analytic step 1 − exp(−t/τ) when the
    /// input is broadband and segments are Hann-windowed.
    #[test]
    fn first_order_lag_plant() {
        let fs = 2000.0f32;
        let tau = 0.02;
        let u = stick_input(40000, fs, 42);
        let y = first_order(&u, fs, tau);
        let step = averaged_step(&u, &y, fs, 4000, Regularization::Constant(1e-4));
        for k in [10usize, 20, 40, 80, 200, 400, 800] {
            let t = k as f32 / fs;
            assert_relative_eq!(step[k], 1.0 - (-t / tau).exp(), epsilon = 0.03);
        }
    }
}
