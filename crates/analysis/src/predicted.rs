//! Predicted post-filter spectrum for an ArduPilot gyro filter chain
//! (`INS_GYRO_FILTER` low-pass + harmonic notches), port of the model in
//! ArduPilot WebTools `FilterReview.js`.

use domain::{ApTune, Spectrum, SpectrumKind};
use dsp::filters::FilterChain;

/// Build the gyro filter chain from parameters. `hover_throttle` (0..1) is
/// used for throttle-mode notch scaling (`MODE = 1`):
/// `freq = FREQ · max(FM_RAT, sqrt(throttle / REF))` — `AP_Vehicle::update_dynamic_notch`
/// (`libraries/AP_Vehicle/AP_Vehicle.cpp`, throttle-based tracking).
pub fn chain_from_params(tune: &ApTune, fs_hz: f64, hover_throttle: Option<f32>, overrides: &[(String, f32)]) -> FilterChain {
    let g = |n: &str| overrides.iter().find(|(k, _)| k == n).map(|(_, v)| *v).or_else(|| tune.get(n));
    let mut c = FilterChain::new(fs_hz as f32);
    if let Some(lpf) = g("INS_GYRO_FILTER") {
        if lpf > 0.0 {
            c.push(dsp::filters::Biquad::ap_lpf2p(lpf, fs_hz as f32));
        }
    }
    for prefix in ["INS_HNTCH_", "INS_HNTC2_"] {
        let p = |n: &str| g(&format!("{prefix}{n}"));
        if p("ENABLE").unwrap_or(0.0) < 0.5 {
            continue;
        }
        let mode = p("MODE").unwrap_or(1.0) as i32;
        let mut freq = p("FREQ").unwrap_or(0.0);
        let bw = p("BW").unwrap_or(freq / 2.0);
        let att = p("ATT").unwrap_or(40.0);
        let hmncs = p("HMNCS").unwrap_or(3.0) as u16;
        let opts = p("OPTS").unwrap_or(0.0) as u32;
        let reference = p("REF").unwrap_or(0.0);
        if mode == 1 {
            if let (Some(thr), true) = (hover_throttle, reference > 0.0) {
                let fm_rat = p("FM_RAT").unwrap_or(1.0).clamp(0.1, 1.0);
                freq = (freq * (thr / reference).sqrt()).max(freq * fm_rat);
            }
        }
        if freq <= 0.0 {
            continue;
        }
        let composite = if opts & 0b1 != 0 { 2 } else if opts & 0b10000 != 0 { 3 } else { 1 };
        c.push_ap_harmonic_notch(freq, bw, att, hmncs, composite);
    }
    c
}

pub fn predict(pre: &Spectrum, chain: &FilterChain) -> Spectrum {
    Spectrum { axis: pre.axis, kind: SpectrumKind::Predicted, f_hz: pre.f_hz.clone(), psd_db: chain.predict_psd_db(&pre.f_hz, &pre.psd_db), nfft: pre.nfft, fs_hz: pre.fs_hz }
}

/// RMS difference in dB between two spectra on the same bins (model sanity check).
pub fn rms_diff_db(a: &Spectrum, b: &Spectrum, f_min: f32, f_max: f32) -> Option<f32> {
    let n = a.f_hz.len().min(b.f_hz.len());
    let mut acc = 0f64;
    let mut c = 0usize;
    for i in 0..n {
        if a.f_hz[i] >= f_min && a.f_hz[i] <= f_max && (a.f_hz[i] - b.f_hz[i]).abs() < 1e-3 {
            acc += ((a.psd_db[i] - b.psd_db[i]) as f64).powi(2);
            c += 1;
        }
    }
    (c > 0).then(|| (acc / c as f64).sqrt() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Axis;

    #[test]
    fn notch_prediction_attenuates_peak() {
        let mut t = ApTune::default();
        for (k, v) in [("INS_GYRO_FILTER", 40.0), ("INS_HNTCH_ENABLE", 1.0), ("INS_HNTCH_MODE", 1.0), ("INS_HNTCH_FREQ", 80.0), ("INS_HNTCH_BW", 40.0), ("INS_HNTCH_ATT", 40.0), ("INS_HNTCH_REF", 0.35), ("INS_HNTCH_HMNCS", 3.0)] {
            t.params.insert(k.into(), v);
        }
        let fs = 2000.0;
        let f_hz: Vec<f32> = (0..500).map(|i| i as f32).collect();
        let pre = Spectrum { axis: Axis::Roll, kind: SpectrumKind::GyroRaw, f_hz: f_hz.clone(), psd_db: vec![0.0; 500], nfft: 4000, fs_hz: fs };
        let chain = chain_from_params(&t, fs, Some(0.35), &[]);
        let p = predict(&pre, &chain);
        assert!(p.psd_db[80] < -30.0, "80 Hz {}", p.psd_db[80]);
        assert!(p.psd_db[160] < -25.0, "160 Hz {}", p.psd_db[160]);
        assert!(p.psd_db[10] > -1.0, "10 Hz {}", p.psd_db[10]);
        // throttle scaling: hover 2x ref -> notch at 80*sqrt(2)=113 Hz
        let chain2 = chain_from_params(&t, fs, Some(0.70), &[]);
        let p2 = predict(&pre, &chain2);
        // 80 Hz now only sees the 40 Hz 2-pole LPF (≈ −12 dB), not the notch
        assert!(p2.psd_db[113] < -25.0 && p2.psd_db[80] > -16.0, "{} {}", p2.psd_db[113], p2.psd_db[80]);
    }
}
