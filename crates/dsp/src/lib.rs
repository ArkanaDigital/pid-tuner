//! Pure numerics. No I/O, no firmware knowledge.
//!
//! Algorithms are ported from (all GPL-3.0, see NOTICES.md):
//! - PIDtoolbox `PTstepcalc.m`, `PTSpec2d.m`, `PTthrSpec.m` (Brian White, via PIDscope)
//! - PID-Analyzer `PID-Analyzer.py` (Florian Melsheimer)
//! - ArduPilot WebTools `Libraries/fft.js`, `FilterReview.js`
//! - Betaflight Configurator `src/js/blackbox/spectral_analysis.js` (chirp frequency response)

pub mod cross;
pub mod decimate;
pub mod fft;
pub mod filters;
pub mod resample;
pub mod welch;
pub mod wiener;
pub mod window;

pub use num_complex::{Complex32, Complex64};

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;

/// Smallest power of two ≥ n.
pub fn next_pow2(n: usize) -> usize {
    n.next_power_of_two()
}

pub fn mean(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    x.iter().sum::<f32>() / x.len() as f32
}

pub fn db10(power: f32) -> f32 {
    10.0 * power.max(1e-30).log10()
}

pub fn db20(amplitude: f32) -> f32 {
    20.0 * amplitude.max(1e-30).log10()
}

/// Percentile (0..=100) of a slice, linear interpolation, copies input.
pub fn percentile(x: &[f32], p: f32) -> f32 {
    if x.is_empty() {
        return f32::NAN;
    }
    let mut v: Vec<f32> = x.iter().copied().filter(|a| a.is_finite()).collect();
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pos = (p.clamp(0.0, 100.0) / 100.0) * (v.len() - 1) as f32;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f32;
    v[lo] * (1.0 - frac) + v[hi] * frac
}
