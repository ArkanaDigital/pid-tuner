//! Discrete filter models used to *predict* the effect of a filter chain on a
//! measured pre-filter spectrum. Coefficients follow the firmware sources:
//!
//! - Betaflight `src/main/common/filter.c`: PT1/PT2/PT3, RBJ biquad LPF (Q=1/√2),
//!   RBJ notch with `Q = f0·fc/(f0² − fc²)`.
//! - ArduPilot `LowPassFilter2p` and `NotchFilter` as modelled in WebTools
//!   `FilterReview.js` (`DigitalBiquadFilter`, `NotchFilter`, `HarmonicNotchFilter`).

use num_complex::Complex32;
use std::f32::consts::{PI, SQRT_2, TAU};

/// Direct-form biquad `H(z) = (b0 + b1 z⁻¹ + b2 z⁻²) / (1 + a1 z⁻¹ + a2 z⁻²)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Complex frequency response at `f` Hz for sample rate `fs`.
    pub fn response(&self, f: f32, fs: f32) -> Complex32 {
        let w = TAU * f / fs;
        let z1 = Complex32::new(w.cos(), -w.sin()); // e^{-jω}
        let z2 = z1 * z1;
        let num = Complex32::new(self.b0, 0.0) + z1 * self.b1 + z2 * self.b2;
        let den = Complex32::new(1.0, 0.0) + z1 * self.a1 + z2 * self.a2;
        num / den
    }

    // ---- Betaflight -------------------------------------------------------

    /// Betaflight `pt1FilterGain`: `k = dt / (RC + dt)`, `RC = 1/(2π f)`.
    pub fn bf_pt1(f_cut: f32, fs: f32) -> Biquad {
        let dt = 1.0 / fs;
        let rc = 1.0 / (TAU * f_cut);
        let k = dt / (rc + dt);
        Biquad {
            b0: k,
            b1: 0.0,
            b2: 0.0,
            a1: -(1.0 - k),
            a2: 0.0,
        }
    }

    /// One PT1 stage of a PT2 (Betaflight applies `cutoffCorrection = 1/√(2^(1/2)−1)`).
    pub fn bf_pt2_stage(f_cut: f32, fs: f32) -> Biquad {
        let corr = 1.0 / (2f32.powf(0.5) - 1.0).sqrt();
        Self::bf_pt1(f_cut * corr, fs)
    }

    /// One PT1 stage of a PT3 (`cutoffCorrection = 1/√(2^(1/3)−1)`).
    pub fn bf_pt3_stage(f_cut: f32, fs: f32) -> Biquad {
        let corr = 1.0 / (2f32.powf(1.0 / 3.0) - 1.0).sqrt();
        Self::bf_pt1(f_cut * corr, fs)
    }

    /// RBJ low-pass with Q = 1/√2 (Betaflight `biquadFilterInit` FILTER_LPF).
    pub fn rbj_lpf(f_cut: f32, fs: f32, q: f32) -> Biquad {
        let w0 = TAU * f_cut / fs;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: ((1.0 - cs) / 2.0) / a0,
            b1: (1.0 - cs) / a0,
            b2: ((1.0 - cs) / 2.0) / a0,
            a1: (-2.0 * cs) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    pub fn bf_biquad_lpf(f_cut: f32, fs: f32) -> Biquad {
        Self::rbj_lpf(f_cut, fs, 1.0 / SQRT_2)
    }

    /// RBJ notch. Betaflight `filterGetNotchQ(center, cutoff) = center·cutoff/(center²−cutoff²)`.
    pub fn rbj_notch(f0: f32, fs: f32, q: f32) -> Biquad {
        let w0 = TAU * f0 / fs;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: 1.0 / a0,
            b1: (-2.0 * cs) / a0,
            b2: 1.0 / a0,
            a1: (-2.0 * cs) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    pub fn bf_notch(center: f32, cutoff: f32, fs: f32) -> Biquad {
        let q = center * cutoff / (center * center - cutoff * cutoff);
        Self::rbj_notch(center, fs, q.max(0.01))
    }

    /// Betaflight dyn-notch / RPM notch: `q` given as firmware integer (e.g. 300 → 3.0 ... no: BF stores Q×100).
    pub fn bf_notch_q(center: f32, q_x100: f32, fs: f32) -> Biquad {
        Self::rbj_notch(center, fs, (q_x100 / 100.0).max(0.01))
    }

    // ---- ArduPilot --------------------------------------------------------

    /// ArduPilot `LowPassFilter2p` (Butterworth-ish second order), as in FilterReview.js.
    pub fn ap_lpf2p(f_cut: f32, fs: f32) -> Biquad {
        if f_cut <= 0.0 {
            return Self::IDENTITY;
        }
        let fr = fs / f_cut;
        let ohm = (PI / fr).tan();
        let c = 1.0 + 2.0 * (PI / 4.0).cos() * ohm + ohm * ohm;
        let b0 = ohm * ohm / c;
        Biquad {
            b0,
            b1: 2.0 * b0,
            b2: b0,
            a1: 2.0 * (ohm * ohm - 1.0) / c,
            a2: (1.0 - 2.0 * (PI / 4.0).cos() * ohm + ohm * ohm) / c,
        }
    }

    /// ArduPilot `NotchFilter::init_with_A_and_Q` as modelled in FilterReview.js:
    /// `A = 10^(−att/40)`, `Q` derived from bandwidth in octaves.
    pub fn ap_notch(center: f32, bandwidth: f32, attenuation_db: f32, fs: f32) -> Biquad {
        if center <= 0.0 || bandwidth <= 0.0 || center >= fs / 2.0 {
            return Self::IDENTITY;
        }
        let a = 10f32.powf(-attenuation_db / 40.0);
        let octaves = 2.0 * (center / (center - bandwidth / 2.0)).max(1.0001).log2();
        let q = (2f32.powf(octaves)).sqrt() / (2f32.powf(octaves) - 1.0);
        let omega = TAU * center / fs;
        let (sn, cs) = omega.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 + alpha * a * a) / a0,
            b1: (-2.0 * cs) / a0,
            b2: (1.0 - alpha * a * a) / a0,
            a1: (-2.0 * cs) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
}

/// A cascade of biquads; response is the product.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterChain {
    pub stages: Vec<Biquad>,
    pub fs: f32,
}

impl FilterChain {
    pub fn new(fs: f32) -> Self {
        Self {
            stages: Vec::new(),
            fs,
        }
    }

    pub fn push(&mut self, b: Biquad) -> &mut Self {
        self.stages.push(b);
        self
    }

    /// Betaflight low-pass by type name.
    pub fn push_bf_lpf(&mut self, kind: BfLpfKind, f_cut: f32) -> &mut Self {
        if f_cut <= 0.0 {
            return self;
        }
        match kind {
            BfLpfKind::Pt1 => self.push(Biquad::bf_pt1(f_cut, self.fs)),
            BfLpfKind::Biquad => self.push(Biquad::bf_biquad_lpf(f_cut, self.fs)),
            BfLpfKind::Pt2 => {
                let s = Biquad::bf_pt2_stage(f_cut, self.fs);
                self.push(s).push(s)
            }
            BfLpfKind::Pt3 => {
                let s = Biquad::bf_pt3_stage(f_cut, self.fs);
                self.push(s).push(s).push(s)
            }
        }
    }

    /// ArduPilot harmonic notch: `harmonics` bitmask (bit n → (n+1)·center),
    /// optional double/triple composite per FilterReview `MultiNotch`.
    pub fn push_ap_harmonic_notch(
        &mut self,
        center: f32,
        bandwidth: f32,
        attenuation_db: f32,
        harmonics_mask: u16,
        composite: u8, // 1 single, 2 double, 3 triple
    ) -> &mut Self {
        for h in 0..16u16 {
            if harmonics_mask & (1 << h) == 0 {
                continue;
            }
            let mul = (h + 1) as f32;
            let fc = center * mul;
            if fc >= self.fs / 2.0 {
                continue;
            }
            match composite.max(1) {
                1 => {
                    self.push(Biquad::ap_notch(
                        fc,
                        bandwidth * mul,
                        attenuation_db,
                        self.fs,
                    ));
                }
                n => {
                    let spread = bandwidth / (32.0 * center);
                    let bw_scaled = bandwidth * mul / n as f32;
                    let offsets: Vec<f32> = match n {
                        2 => vec![1.0 - spread, 1.0 + spread],
                        _ => vec![1.0 - spread, 1.0, 1.0 + spread],
                    };
                    for o in offsets {
                        self.push(Biquad::ap_notch(fc * o, bw_scaled, attenuation_db, self.fs));
                    }
                }
            }
        }
        self
    }

    pub fn response(&self, f: f32) -> Complex32 {
        self.stages.iter().fold(Complex32::new(1.0, 0.0), |acc, b| {
            acc * b.response(f, self.fs)
        })
    }

    pub fn magnitude_db(&self, f: f32) -> f32 {
        20.0 * self.response(f).norm().max(1e-12).log10()
    }

    /// Phase lag in degrees (positive = lag).
    pub fn phase_lag_deg(&self, f: f32) -> f32 {
        -self.response(f).arg().to_degrees()
    }

    /// Predicted post-filter PSD: `pre_db + 20·log10|H(f)|` per bin.
    pub fn predict_psd_db(&self, f_hz: &[f32], pre_db: &[f32]) -> Vec<f32> {
        f_hz.iter()
            .zip(pre_db)
            .map(|(f, p)| p + self.magnitude_db(*f))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BfLpfKind {
    Pt1,
    Biquad,
    Pt2,
    Pt3,
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn pt1_is_minus_3db_at_cutoff() {
        let fs = 8000.0;
        let b = Biquad::bf_pt1(100.0, fs);
        let db = 20.0 * b.response(100.0, fs).norm().log10();
        // Backward-Euler PT1 is slightly more damped than the analog −3.01 dB.
        assert_relative_eq!(db, -3.01, epsilon = 0.3);
        assert_relative_eq!(b.response(0.0, fs).norm(), 1.0, epsilon = 1e-5);
    }

    #[test]
    fn pt2_and_pt3_are_minus_3db_at_cutoff() {
        let fs = 8000.0;
        let mut c2 = FilterChain::new(fs);
        c2.push_bf_lpf(BfLpfKind::Pt2, 150.0);
        // Backward-Euler stages at fs/f_cut ≈ 53 are ~0.3 dB more damped each.
        assert_relative_eq!(c2.magnitude_db(150.0), -3.01, epsilon = 0.8);
        let mut c3 = FilterChain::new(fs);
        c3.push_bf_lpf(BfLpfKind::Pt3, 150.0);
        assert_relative_eq!(c3.magnitude_db(150.0), -3.01, epsilon = 1.0);
    }

    #[test]
    fn rbj_lpf_butterworth() {
        let fs = 8000.0;
        let b = Biquad::bf_biquad_lpf(200.0, fs);
        let db = 20.0 * b.response(200.0, fs).norm().log10();
        assert_relative_eq!(db, -3.01, epsilon = 0.1);
    }

    #[test]
    fn bf_notch_is_deep_at_center() {
        let fs = 8000.0;
        let b = Biquad::bf_notch(200.0, 160.0, fs);
        assert!(20.0 * b.response(200.0, fs).norm().log10() < -40.0);
        assert_relative_eq!(b.response(50.0, fs).norm(), 1.0, epsilon = 0.05);
    }

    #[test]
    fn ap_lpf2p_cutoff() {
        let fs = 1000.0;
        let b = Biquad::ap_lpf2p(40.0, fs);
        let db = 20.0 * b.response(40.0, fs).norm().log10();
        assert_relative_eq!(db, -3.01, epsilon = 0.2);
    }

    #[test]
    fn ap_notch_attenuation() {
        let fs = 1000.0;
        let b = Biquad::ap_notch(80.0, 40.0, 40.0, fs);
        let db = 20.0 * b.response(80.0, fs).norm().log10();
        assert_relative_eq!(db, -40.0, epsilon = 0.5);
    }

    #[test]
    fn harmonic_notch_hits_harmonics() {
        let fs = 1000.0;
        let mut c = FilterChain::new(fs);
        c.push_ap_harmonic_notch(80.0, 40.0, 40.0, 0b111, 1);
        for f in [80.0, 160.0, 240.0] {
            assert!(c.magnitude_db(f) < -30.0, "f={f}");
        }
        assert!(c.magnitude_db(20.0) > -1.0);
    }
}
