//! Thin FFT wrappers around rustfft/realfft with cached plans.

use num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Real → complex forward FFT (one-sided, `n/2+1` bins, unnormalized like MATLAB/numpy).
pub struct RealFft {
    n: usize,
    plan: Arc<dyn RealToComplex<f32>>,
    scratch: Vec<Complex32>,
}

impl RealFft {
    pub fn new(n: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let plan = planner.plan_fft_forward(n);
        let scratch = plan.make_scratch_vec();
        Self { n, plan, scratch }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn out_len(&self) -> usize {
        self.n / 2 + 1
    }

    /// `input` is consumed (realfft scrambles it). Returns `n/2+1` bins.
    pub fn forward(&mut self, input: &mut [f32]) -> Vec<Complex32> {
        let mut out = self.plan.make_output_vec();
        self.plan
            .process_with_scratch(input, &mut out, &mut self.scratch)
            .expect("realfft length mismatch");
        out
    }

    /// Bin center frequencies in Hz for a sample rate `fs`.
    pub fn freqs(&self, fs: f64) -> Vec<f32> {
        let n = self.n as f64;
        (0..self.out_len()).map(|k| (k as f64 * fs / n) as f32).collect()
    }
}

/// Complex ↔ complex FFT pair (unnormalized forward, inverse divided by n).
pub struct ComplexFft {
    n: usize,
    fwd: Arc<dyn Fft<f32>>,
    inv: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
}

impl ComplexFft {
    pub fn new(n: usize) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let len = fwd.get_inplace_scratch_len().max(inv.get_inplace_scratch_len());
        Self {
            n,
            fwd,
            inv,
            scratch: vec![Complex32::default(); len],
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn forward_real(&mut self, x: &[f32]) -> Vec<Complex32> {
        let mut buf: Vec<Complex32> = x.iter().map(|v| Complex32::new(*v, 0.0)).collect();
        buf.resize(self.n, Complex32::default());
        self.fwd.process_with_scratch(&mut buf, &mut self.scratch);
        buf
    }

    /// Inverse FFT, normalized by 1/n, real part returned.
    pub fn inverse_real(&mut self, mut spec: Vec<Complex32>) -> Vec<f32> {
        debug_assert_eq!(spec.len(), self.n);
        self.inv.process_with_scratch(&mut spec, &mut self.scratch);
        let inv_n = 1.0 / self.n as f32;
        spec.iter().map(|c| c.re * inv_n).collect()
    }

    /// Two-sided bin frequencies (numpy `fftfreq` layout), absolute value.
    pub fn abs_freqs(&self, fs: f64) -> Vec<f32> {
        let n = self.n;
        (0..n)
            .map(|k| {
                let k = if k <= n / 2 { k as f64 } else { k as f64 - n as f64 };
                (k.abs() * fs / n as f64) as f32
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn roundtrip_complex() {
        let x: Vec<f32> = (0..64).map(|i| (i as f32 * 0.3).sin()).collect();
        let mut f = ComplexFft::new(64);
        let spec = f.forward_real(&x);
        let y = f.inverse_real(spec);
        for (a, b) in x.iter().zip(&y) {
            assert_relative_eq!(a, b, epsilon = 1e-4);
        }
    }

    #[test]
    fn real_fft_sine_peak() {
        let n = 1024;
        let fs = 1000.0;
        let mut x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 125.0 * i as f32 / fs as f32).sin())
            .collect();
        let mut f = RealFft::new(n);
        let spec = f.forward(&mut x);
        let (k, _) = spec
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.norm().partial_cmp(&b.1.norm()).unwrap())
            .unwrap();
        assert_eq!(k, 128); // 125 Hz * 1024 / 1000
        assert_relative_eq!(spec[128].norm(), n as f32 / 2.0, epsilon = 1.0);
    }
}
