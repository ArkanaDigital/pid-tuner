//! Step response via Wiener deconvolution of setpoint → filtered gyro.
//!
//! Faithful port of PIDtoolbox `PTstepcalc.m` (Brian White, Beer-Ware/GPL via
//! the ianrmurphy/skoch1s forks) as the default variant, plus the PID-Analyzer
//! / ArduPilot PIDReview regularisation as an alternative.

use domain::{Axis, FlightLog, StepResponse, StepVariant};
use dsp::wiener::{cumsum, impulse_response, Regularization};
use dsp::window::{hamming, hann, moving_average};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Taper {
    None,
    Hann,
    Hamming,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOpts {
    pub variant: StepVariant,
    /// Segment length in seconds (PIDtoolbox 2.0, PID-Analyzer 1.0).
    pub segment_s: f32,
    /// Hop between segments in seconds (PIDtoolbox: segment/subsamp).
    pub hop_s: f32,
    /// Response window in seconds (0.5).
    pub response_s: f32,
    /// Segments whose max |setpoint| is below this (deg/s) are skipped (PIDtoolbox 20).
    pub min_input_dps: f32,
    pub taper: Taper,
    /// Moving-average width applied to the impulse response, in ms (PIDtoolbox 10).
    pub smooth_ms: f32,
    /// Zero padding samples appended before the FFT (PIDtoolbox 0, PID-Analyzer pads to 1024).
    pub pad_samples: usize,
    /// PIDtoolbox `stepinfo` quality control: after the response has risen,
    /// min ∈ (0.5, 1] and max ∈ (1, 2).
    pub quality_control: bool,
    /// PIDtoolbox `Ycorrection`: normalise each segment so its steady state is 1.0.
    pub y_correction: bool,
    /// Only use segments with max |setpoint| ≥ this (PIDtoolbox `rateHigh` split at 500).
    pub high_rate_only: bool,
}

impl Default for StepOpts {
    fn default() -> Self {
        Self {
            variant: StepVariant::PtStep,
            segment_s: 2.0,
            hop_s: 0.2,
            response_s: 0.5,
            min_input_dps: 20.0,
            taper: Taper::Hamming,
            smooth_ms: 10.0,
            pad_samples: 0,
            quality_control: true,
            y_correction: false,
            high_rate_only: false,
        }
    }
}

impl StepOpts {
    pub fn pid_analyzer() -> Self {
        Self {
            variant: StepVariant::PidAnalyzer,
            segment_s: 1.0,
            hop_s: 1.0 / 16.0,
            taper: Taper::Hann,
            smooth_ms: 0.0,
            pad_samples: 1024,
            ..Default::default()
        }
    }
}

/// MATLAB-`stepinfo`-like quality gate used by PIDtoolbox.
/// Returns `(settling_min, settling_max)` over the part of the response after it
/// has risen to 90 % of its final value.
fn settling_extrema(y: &[f32]) -> Option<(f32, f32)> {
    let yfinal = *y.last()?;
    if !yfinal.is_finite() {
        return None;
    }
    let rise_end = y.iter().position(|v| *v >= 0.9 * yfinal)?;
    let tail = &y[rise_end..];
    let (mn, mx) = tail.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    Some((mn, mx))
}

/// Compute the averaged step response of one axis. Returns a response with
/// `n_segments == 0` when nothing usable was found.
pub fn step_response(log: &FlightLog, axis: Axis, opts: &StepOpts) -> StepResponse {
    let fs = log.fs_hz as f32;
    let ax = log.axis(axis);
    let sp = &ax.setpoint;
    let gy = &ax.gyro_filt;
    let n = sp.len().min(gy.len());

    let seg = ((opts.segment_s * fs) as usize).max(16);
    let hop = ((opts.hop_s * fs) as usize).max(1);
    let wnd = ((opts.response_s * fs) as usize).max(2);
    let pad = if opts.pad_samples > 0 {
        // PID-Analyzer pads up to the next multiple of 1024.
        seg.div_ceil(opts.pad_samples) * opts.pad_samples
    } else {
        seg
    };
    let reg = match opts.variant {
        StepVariant::PtStep => Regularization::Constant(1e-4),
        StepVariant::PidAnalyzer => Regularization::Gaussian { cutoff_hz: 25.0 },
    };
    let window = match opts.taper {
        Taper::None => None,
        Taper::Hann => Some(hann(seg)),
        Taper::Hamming => Some(hamming(seg)),
    };
    let smooth_w = ((opts.smooth_ms * fs / 1000.0).round() as usize).max(1);

    let mut responses: Vec<Vec<f32>> = Vec::new();
    let mut rejected = 0usize;
    let mut start = 0usize;
    while n >= seg && start + seg <= n {
        let u = &sp[start..start + seg];
        let y = &gy[start..start + seg];
        let max_in = u.iter().fold(0f32, |m, v| m.max(v.abs()));
        if max_in < opts.min_input_dps || (opts.high_rate_only && max_in < 500.0) {
            start += hop;
            continue;
        }
        let (uw, yw): (Vec<f32>, Vec<f32>) = match &window {
            Some(w) => (
                u.iter().zip(w).map(|(a, b)| a * b).collect(),
                y.iter().zip(w).map(|(a, b)| a * b).collect(),
            ),
            None => (u.to_vec(), y.to_vec()),
        };
        let imp = impulse_response(&uw, &yw, log.fs_hz, pad, reg);
        let imp = if smooth_w > 1 { moving_average(&imp, smooth_w) } else { imp };
        let mut step = cumsum(&imp[..(wnd + 1).min(imp.len())]);
        let first = step[0];
        for v in step.iter_mut() {
            *v -= first;
        }
        step.truncate(wnd);

        if opts.quality_control {
            match settling_extrema(&step) {
                Some((mn, mx)) if mn > 0.5 && mn <= 1.0 && mx > 1.0 && mx < 2.0 => {}
                _ => {
                    rejected += 1;
                    start += hop;
                    continue;
                }
            }
        }
        if opts.y_correction {
            let ss_n = ((0.1 * fs) as usize).clamp(1, step.len());
            let ss = step[step.len() - ss_n..].iter().sum::<f32>() / ss_n as f32;
            if ss.abs() > 1e-6 {
                for v in step.iter_mut() {
                    *v /= ss;
                }
            }
        }
        responses.push(step);
        start += hop;
    }

    let t_ms: Vec<f32> = (0..wnd).map(|i| i as f32 * 1000.0 / fs).collect();
    let n_segments = responses.len();
    let (mean, p10, p90) = if n_segments == 0 {
        (vec![0.0; wnd], vec![0.0; wnd], vec![0.0; wnd])
    } else {
        let mut mean = vec![0f32; wnd];
        let mut p10 = vec![0f32; wnd];
        let mut p90 = vec![0f32; wnd];
        let mut col = Vec::with_capacity(n_segments);
        for k in 0..wnd {
            col.clear();
            col.extend(responses.iter().map(|r| r[k]));
            mean[k] = col.iter().sum::<f32>() / n_segments as f32;
            p10[k] = dsp::percentile(&col, 10.0);
            p90[k] = dsp::percentile(&col, 90.0);
        }
        (mean, p10, p90)
    };

    let m = metrics(&mean, fs);
    StepResponse {
        axis,
        variant: opts.variant,
        t_ms,
        mean,
        p10,
        p90,
        n_segments,
        rejected,
        overshoot: m.overshoot,
        latency_ms: m.latency_ms,
        settle_ms: m.settle_ms,
        steady_state: m.steady_state,
    }
}

pub struct StepMetrics {
    pub overshoot: f32,
    pub latency_ms: f32,
    pub settle_ms: Option<f32>,
    pub steady_state: f32,
}

/// PIDtoolbox `PTtuningParams.m`: peak within the first 150 ms; latency = first
/// crossing of 0.5 (their code subtracts 1 ms; we report the raw crossing).
pub fn metrics(mean: &[f32], fs: f32) -> StepMetrics {
    let n = mean.len();
    if n == 0 {
        return StepMetrics { overshoot: 0.0, latency_ms: 0.0, settle_ms: None, steady_state: 0.0 };
    }
    let i150 = ((0.15 * fs) as usize).min(n);
    let overshoot = mean[..i150.max(1)].iter().cloned().fold(f32::MIN, f32::max);
    let latency_ms = mean
        .iter()
        .position(|v| *v > 0.5)
        .map(|i| i as f32 * 1000.0 / fs)
        .unwrap_or(f32::NAN);
    let ss_n = ((0.1 * fs) as usize).clamp(1, n);
    let steady_state = mean[n - ss_n..].iter().sum::<f32>() / ss_n as f32;
    let band = 0.05 * steady_state.abs().max(1e-3);
    let settle_ms = if steady_state.abs() > 1e-3 {
        let last_out = mean.iter().rposition(|v| (v - steady_state).abs() > band);
        match last_out {
            None => Some(0.0),
            Some(i) if i + 1 < n => Some((i + 1) as f32 * 1000.0 / fs),
            Some(_) => None,
        }
    } else {
        None
    };
    StepMetrics { overshoot, latency_ms, settle_ms, steady_state }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use domain::*;

    fn lcg(seed: &mut u64) -> f32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }

    /// Synthetic log: stick-like setpoint, second-order underdamped plant with
    /// known overshoot, plus gyro noise, so the estimator can be checked.
    fn synthetic_log(fs: f32, secs: f32, noise_dps: f32) -> FlightLog {
        let n = (fs * secs) as usize;
        let mut seed = 99u64;
        let mut sp = vec![0f32; n];
        let mut i = 0;
        while i < n {
            let hold = (0.15 * fs + 0.3 * fs * (lcg(&mut seed) * 0.5 + 0.5)) as usize;
            let level = 400.0 * lcg(&mut seed);
            for v in sp.iter_mut().skip(i).take(hold) {
                *v = level;
            }
            i += hold.max(1);
        }
        let sa = (-1.0 / (0.005 * fs)).exp();
        for i in 1..n {
            sp[i] = sa * sp[i - 1] + (1.0 - sa) * sp[i];
        }
        // Second-order: wn = 2π·12 Hz, zeta = 0.5 → overshoot ≈ 16 %.
        let wn = std::f32::consts::TAU * 12.0;
        let zeta = 0.5;
        let dt = 1.0 / fs;
        let mut y = vec![0f32; n];
        let mut v = 0f32;
        for i in 1..n {
            let acc = wn * wn * (sp[i] - y[i - 1]) - 2.0 * zeta * wn * v;
            v += acc * dt;
            y[i] = y[i - 1] + v * dt;
        }
        for yi in y.iter_mut() {
            *yi += noise_dps * lcg(&mut seed);
        }
        let t: Vec<f32> = (0..n).map(|i| i as f32 / fs).collect();
        let mut axes: [AxisSeries; 3] = Default::default();
        axes[0].setpoint = sp;
        axes[0].gyro_filt = y;
        FlightLog {
            id: LogId("test".into()),
            firmware: Firmware::Unknown { product: "synthetic".into() },
            fs_hz: fs as f64,
            t,
            axes,
            motors: vec![],
            throttle: vec![0.5; n],
            erpm: None,
            gaps: vec![],
            meta: LogMeta::default(),
            tune_at_log: Tune::Unknown,
        }
    }

    #[test]
    fn recovers_second_order_overshoot() {
        let log = synthetic_log(2000.0, 30.0, 0.0);
        let r = step_response(&log, Axis::Roll, &StepOpts::default());
        assert!(r.n_segments > 20, "segments {} rejected {}", r.n_segments, r.rejected);
        // analytic: overshoot exp(-πζ/√(1-ζ²)) = 0.163 (10 ms smoothing shaves a little)
        assert_relative_eq!(r.overshoot, 1.163, epsilon = 0.05);
        assert_relative_eq!(r.steady_state, 1.0, epsilon = 0.04);
        // analytic 50 % crossing for ζ=0.5, ωn=75.4 rad/s ≈ 15 ms
        assert!(r.latency_ms > 10.0 && r.latency_ms < 24.0, "latency {}", r.latency_ms);
    }

    #[test]
    fn robust_to_gyro_noise() {
        let log = synthetic_log(2000.0, 40.0, 15.0);
        let r = step_response(&log, Axis::Roll, &StepOpts::default());
        assert!(r.n_segments > 10, "segments {} rejected {}", r.n_segments, r.rejected);
        assert_relative_eq!(r.steady_state, 1.0, epsilon = 0.08);
        assert!(r.overshoot > 1.05 && r.overshoot < 1.3, "overshoot {}", r.overshoot);
    }

    #[test]
    fn pid_analyzer_variant_agrees() {
        let log = synthetic_log(2000.0, 30.0, 0.0);
        let r = step_response(&log, Axis::Roll, &StepOpts::pid_analyzer());
        assert!(r.n_segments > 20);
        assert_relative_eq!(r.steady_state, 1.0, epsilon = 0.08);
    }

    #[test]
    fn no_input_gives_zero_segments() {
        let mut log = synthetic_log(2000.0, 5.0, 0.0);
        log.axes[0].setpoint.iter_mut().for_each(|v| *v = 0.0);
        let r = step_response(&log, Axis::Roll, &StepOpts::default());
        assert_eq!(r.n_segments, 0);
    }
}
