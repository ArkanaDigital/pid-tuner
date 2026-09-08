//! Closed-loop frequency response from Betaflight CHIRP sweeps.
//!
//! Port of betaflight-configurator `src/js/blackbox/spectral_analysis.js`
//! (GPL-3.0-or-later): H = setpoint→gyro via Welch cross-spectra, coherence,
//! open loop L = H/(1−H), sensitivity S = 1−H, and the scalar metrics of the
//! Configurator Autotune tab. Differences (deliberate): windows are detrended,
//! and nothing here derives a filter setting from coherence (Configurator issue
//! #5258); recommendations are gated on coherence in `recommend`.

use domain::*;
use dsp::cross::{self, CrossSpectrum, CsdOpts};
use dsp::Complex32;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChirpOpts {
    pub detrend: bool,
    pub overlap: f32,
    /// Open loop only above this frequency (Configurator MIN_OPEN_LOOP_HZ).
    pub ol_min_hz: f32,
    /// … and only where coherence is at least this (CROSSOVER_COHERENCE_MIN).
    pub ol_min_coh: f32,
    /// Coherence gate for bandwidth / resonance / sensitivity readings.
    pub bw_min_coh: f32,
    pub max_hz: f32,
    pub delay_band_hz: (f32, f32),
    pub lf_band_hz: (f32, f32),
    pub coh_band_hz: (f32, f32),
    pub target_pm_deg: Vec<f32>,
    pub sens_limit: f32,
    pub gain_scan: (f32, f32, f32),
    pub step_ms: f32,
}

impl Default for ChirpOpts {
    fn default() -> Self {
        Self {
            detrend: true,
            overlap: 0.5,
            ol_min_hz: 2.0,
            ol_min_coh: 0.5,
            bw_min_coh: 0.3,
            max_hz: 500.0,
            delay_band_hz: (20.0, 140.0),
            lf_band_hz: (2.0, 10.0),
            coh_band_hz: (5.0, 100.0),
            target_pm_deg: vec![50.0, 60.0, 72.5],
            sens_limit: 2.0,
            gain_scan: (0.5, 2.0, 0.01),
            step_ms: 100.0,
        }
    }
}

fn db20(x: f32) -> f32 {
    if x.is_finite() && x > 0.0 {
        20.0 * x.log10()
    } else {
        f32::NAN
    }
}

/// Linear interpolation of `y` at `x` on the ascending grid `f` (NaN outside / invalid).
fn interp_at(f: &[f32], y: &[f32], x: f32) -> f32 {
    if !x.is_finite() || f.len() < 2 {
        return f32::NAN;
    }
    for k in 1..f.len() {
        if f[k] >= x {
            if !(y[k].is_finite() && y[k - 1].is_finite()) {
                return f32::NAN;
            }
            let fr = (x - f[k - 1]) / (f[k] - f[k - 1]);
            return y[k - 1] + fr * (y[k] - y[k - 1]);
        }
    }
    f32::NAN
}

fn lsq_slope(x: &[f32], y: &[f32]) -> f32 {
    let n = x.len() as f32;
    if n < 5.0 {
        return f32::NAN;
    }
    let mx = x.iter().sum::<f32>() / n;
    let my = y.iter().sum::<f32>() / n;
    let (mut sxy, mut sxx) = (0.0f32, 0.0f32);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
    }
    if sxx > 0.0 {
        sxy / sxx
    } else {
        f32::NAN
    }
}

/// Number of sweeps inside `debug2[i0..i1]` (frequency restarts + 1).
fn count_sweeps(debug2: Option<&[f32]>, i0: usize, i1: usize) -> usize {
    let Some(d) = debug2 else { return 1 };
    let mut restarts = 0usize;
    for w in d[i0..i1].windows(2) {
        if w[1] + 100.0 < w[0] {
            restarts += 1;
        }
    }
    restarts + 1
}

pub fn frequency_response(
    log: &FlightLog,
    axis: Axis,
    opts: &ChirpOpts,
) -> Option<FrequencyResponse> {
    let info = log.chirp.as_ref()?;
    let fs = log.fs_hz;
    let nfft = cross::segment_size_for(fs);
    let mut acc = CrossSpectrum::empty(nfft, fs);
    let ax = &log.axes[axis.index()];
    let mut n_sweeps = 0usize;
    let mut sweep_s = 0.0f32;
    // Rate-loop (ACRO) sweeps first; fall back to ANGLE-mode sweeps, flagged.
    let has_acro = info.segments_for(axis).any(|s| !s.angle_mode);
    let angle_mode = !has_acro;
    for seg in info
        .segments_for(axis)
        .filter(|s| s.angle_mode == angle_mode)
    {
        if seg.i1 <= seg.i0 || seg.i1 - seg.i0 < nfft || seg.i1 > ax.setpoint.len() {
            continue;
        }
        let u = &ax.setpoint[seg.i0..seg.i1];
        let y = &ax.gyro_filt[seg.i0..seg.i1];
        let cs = cross::cross_welch(
            u,
            y,
            fs,
            CsdOpts {
                nfft,
                overlap: opts.overlap,
                detrend: opts.detrend,
            },
        );
        acc.accumulate(&cs);
        n_sweeps += count_sweeps(log.debug.get(2).map(|v| v.as_slice()), seg.i0, seg.i1);
        sweep_s += seg.t1_s - seg.t0_s;
    }
    if acc.n_windows == 0 {
        return None;
    }
    let f = acc.f_hz.clone();
    let nb = f.len();
    let h = acc.transfer();
    let coh = acc.coherence();
    let h_mag_db: Vec<f32> = h.iter().map(|c| db20(c.norm())).collect();
    let h_phase_deg: Vec<f32> = h
        .iter()
        .map(|c| {
            if c.re.is_finite() {
                c.arg().to_degrees()
            } else {
                f32::NAN
            }
        })
        .collect();

    // open loop where coherent and above ol_min_hz
    let l_valid: Vec<bool> = (0..nb)
        .map(|k| f[k] >= opts.ol_min_hz && coh[k] >= opts.ol_min_coh && h[k].re.is_finite())
        .collect();
    let l_all = cross::open_loop(&h);
    let l: Vec<Complex32> = (0..nb)
        .map(|k| {
            if l_valid[k] {
                l_all[k]
            } else {
                Complex32::new(f32::NAN, f32::NAN)
            }
        })
        .collect();
    let l_mag: Vec<f32> = l
        .iter()
        .map(|c| if c.re.is_finite() { c.norm() } else { f32::NAN })
        .collect();
    let l_mag_db: Vec<f32> = l_mag.iter().map(|m| db20(*m)).collect();
    let mut l_phase_deg: Vec<f32> = l
        .iter()
        .map(|c| {
            if c.re.is_finite() {
                c.arg().to_degrees()
            } else {
                f32::NAN
            }
        })
        .collect();
    cross::unwrap_deg(&mut l_phase_deg);
    let s = cross::sensitivity(&h);
    let s_mag_db: Vec<f32> = s.iter().map(|c| db20(c.norm())).collect();

    // ---- metrics --------------------------------------------------------------
    let bw_valid: Vec<bool> = (0..nb)
        .map(|k| coh[k] >= opts.bw_min_coh && h_mag_db[k].is_finite())
        .collect();
    let bandwidth_hz = cross::interp_crossing(&f, &h_mag_db, -3.0, &bw_valid, true);
    let crossover_hz = cross::interp_crossing(&f, &l_mag, 1.0, &l_valid, true);
    let phase_margin_deg = if crossover_hz.is_finite() {
        180.0 + interp_at(&f, &l_phase_deg, crossover_hz)
    } else {
        f32::NAN
    };
    let max_phase_margin_deg = l_phase_deg
        .iter()
        .zip(&l_valid)
        .filter(|(p, v)| **v && p.is_finite())
        .map(|(p, _)| 180.0 + *p)
        .fold(f32::NAN, f32::max);
    let in_band = |k: usize, lo: f32, hi: f32| f[k] > lo && f[k] < hi;
    let (mut mr_db, mut mr_hz) = (f32::NAN, f32::NAN);
    let (mut sp_db, mut sp_hz) = (f32::NAN, f32::NAN);
    for k in 0..nb {
        if in_band(k, 0.0, opts.max_hz) && coh[k] >= opts.bw_min_coh {
            if h_mag_db[k].is_finite() && (mr_db.is_nan() || h_mag_db[k] > mr_db) {
                mr_db = h_mag_db[k];
                mr_hz = f[k];
            }
            if s_mag_db[k].is_finite() && (sp_db.is_nan() || s_mag_db[k] > sp_db) {
                sp_db = s_mag_db[k];
                sp_hz = f[k];
            }
        }
    }
    let (dx, dy): (Vec<f32>, Vec<f32>) = (0..nb)
        .filter(|&k| {
            l_valid[k]
                && in_band(k, opts.delay_band_hz.0, opts.delay_band_hz.1)
                && l_phase_deg[k].is_finite()
        })
        .map(|k| (f[k], l_phase_deg[k]))
        .unzip();
    let loop_delay_ms = -lsq_slope(&dx, &dy) / 360.0 * 1000.0;
    let lf: Vec<f32> = (0..nb)
        .filter(|&k| {
            in_band(k, opts.lf_band_hz.0, opts.lf_band_hz.1)
                && coh[k] > opts.bw_min_coh
                && h_mag_db[k].is_finite()
        })
        .map(|k| h_mag_db[k])
        .collect();
    let low_freq_err_db = if lf.is_empty() {
        f32::NAN
    } else {
        lf.iter().sum::<f32>() / lf.len() as f32
    };
    let cb: Vec<f32> = (0..nb)
        .filter(|&k| in_band(k, opts.coh_band_hz.0, opts.coh_band_hz.1))
        .map(|k| coh[k])
        .collect();
    let coherence_mean = if cb.is_empty() {
        f32::NAN
    } else {
        cb.iter().sum::<f32>() / cb.len() as f32
    };
    let mut noise_floor_hz = f32::NAN;
    let mut ever = false;
    for k in 1..nb {
        if coh[k] >= 0.5 {
            ever = true;
        } else if ever && f[k] > 20.0 {
            noise_floor_hz = f[k];
            break;
        }
    }
    if noise_floor_hz.is_nan() && ever {
        noise_floor_hz = f[nb - 1];
    }
    // targets
    let targets: Vec<FrTarget> = opts
        .target_pm_deg
        .iter()
        .map(|&pm| {
            let wanted = -(180.0 - pm);
            let cx = cross::interp_crossing(&f, &l_phase_deg, wanted, &l_valid, true);
            let mag = interp_at(&f, &l_mag, cx);
            let gain_to_target = if mag.is_finite() && mag > 0.0 {
                1.0 / mag
            } else {
                f32::NAN
            };
            let (g0, g1, dg) = opts.gain_scan;
            let mut g = g1;
            let mut best = f32::NAN;
            while g >= g0 - 1e-6 {
                if cross::predicted_sens_peak(&l, g) <= opts.sens_limit {
                    best = g;
                    break;
                }
                g -= dg;
            }
            FrTarget {
                pm_deg: pm,
                crossover_hz: cx,
                gain_to_target,
                gain_for_sens_limit: best,
            }
        })
        .collect();
    // step from H
    let (step_t_ms, step) = cross::step_from_h(&h, nfft, fs, opts.step_ms);
    let (step_overshoot, step_rise_ms, step_settle_ms) = step_metrics(&step_t_ms, &step);

    Some(FrequencyResponse {
        axis,
        angle_mode,
        f_hz: f,
        h_mag_db,
        h_phase_deg,
        coherence: coh,
        l_mag_db,
        l_phase_deg,
        s_mag_db,
        step_t_ms,
        step,
        fs_hz: fs,
        segment_size: nfft,
        n_windows: acc.n_windows,
        n_sweeps,
        sweep_seconds: if n_sweeps > 0 {
            sweep_s / n_sweeps as f32
        } else {
            0.0
        },
        metrics: FrMetrics {
            bandwidth_hz,
            crossover_hz,
            phase_margin_deg,
            max_phase_margin_deg,
            resonant_peak_db: mr_db,
            resonant_peak_hz: mr_hz,
            loop_delay_ms,
            low_freq_err_db,
            coherence_mean,
            noise_floor_hz,
            sens_peak_db: sp_db,
            sens_peak_hz: sp_hz,
            step_overshoot,
            step_rise_ms,
            step_settle_ms,
            targets,
        },
    })
}

/// Overshoot as peak/steady-state (like `StepResponse::overshoot`), 10→90 % rise, ±5 % settle.
fn step_metrics(t: &[f32], s: &[f32]) -> (f32, f32, f32) {
    let n = s.len();
    if n < 10 {
        return (f32::NAN, f32::NAN, f32::NAN);
    }
    let tail = &s[n - n / 10..];
    let ss = tail.iter().sum::<f32>() / tail.len() as f32;
    if !ss.is_finite() || ss.abs() < 1e-6 {
        return (f32::NAN, f32::NAN, f32::NAN);
    }
    let peak = s.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let overshoot = peak / ss;
    let t10 = t
        .iter()
        .zip(s)
        .find(|(_, v)| **v >= 0.1 * ss)
        .map(|(t, _)| *t);
    let t90 = t
        .iter()
        .zip(s)
        .find(|(_, v)| **v >= 0.9 * ss)
        .map(|(t, _)| *t);
    let rise = match (t10, t90) {
        (Some(a), Some(b)) => b - a,
        _ => f32::NAN,
    };
    let settle = s
        .iter()
        .enumerate()
        .rev()
        .find(|(_, v)| ((**v - ss) / ss).abs() > 0.05)
        .map(|(i, _)| t[(i + 1).min(n - 1)])
        .unwrap_or(0.0);
    (overshoot, rise, settle)
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// Exact open loop of the discrete simulator in `simulate`: first-order
    /// `v[n] = a v[n-1] + (1-a) K u[n]`, integrator `y[n] = y[n-1] + dt v[n]`,
    /// input delayed by `d` samples. Returns (|L|, ∠L in degrees).
    pub(crate) fn l_analytic(f: f64, k: f64, tau: f64, td: f64) -> (f64, f64) {
        let fs = 4000.0f64;
        let dt = 1.0 / fs;
        let a = (-dt / tau).exp();
        let d = (td * fs).round();
        let w = std::f64::consts::TAU * f * dt;
        let z1 = dsp::Complex64::from_polar(1.0, -w); // z^-1
        let one = dsp::Complex64::new(1.0, 0.0);
        let first = (one - a) * k / (one - a * z1);
        let integ = dt / (one - z1);
        let delay = dsp::Complex64::from_polar(1.0, -w * d);
        let l = first * integ * delay;
        (l.norm(), l.arg().to_degrees())
    }

    pub(crate) fn h_from_l(mag: f64, ph_deg: f64) -> (f64, f64) {
        let l = dsp::Complex64::from_polar(mag, ph_deg.to_radians());
        let h = l / (dsp::Complex64::new(1.0, 0.0) + l);
        (h.norm(), h.arg().to_degrees())
    }

    /// Simulate the closed loop with a chirp reference; returns (setpoint, gyro).
    pub(crate) fn simulate(
        fs: f64,
        secs: f64,
        sweeps: usize,
        k: f64,
        tau: f64,
        td: f64,
        noise_amp: f32,
    ) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let dt = 1.0 / fs;
        let n_per = (secs * fs) as usize;
        let n = n_per * sweeps;
        let delay = (td * fs).round() as usize;
        let a = (-dt / tau).exp();
        let (f0, f1) = (0.2f64, 600.0f64);
        let mut r = Vec::with_capacity(n);
        let mut d2 = Vec::with_capacity(n);
        for i in 0..n {
            let t = (i % n_per) as f64 * dt;
            let f = f0 * (f1 / f0).powf(t / secs);
            let phase = std::f64::consts::TAU * f0 * secs / (f1 / f0).ln()
                * ((f1 / f0).powf(t / secs) - 1.0);
            r.push((230.0 * phase.sin()) as f32);
            d2.push((f * 10.0) as f32);
        }
        let mut y = vec![0f32; n];
        let mut v = 0.0f64;
        let mut yy = 0.0f64;
        assert!(
            delay >= 1,
            "the loop needs at least one sample of delay to be causal"
        );
        let mut ubuf = vec![0.0f64; delay + 1];
        let mut seed = 0x1234_5678_9abc_def0u64;
        for i in 0..n {
            // unity feedback with a pure delay of `delay` samples in the loop:
            // u[i] = e[i-delay]; y[i] from u[i]; then e[i] = r[i] - y[i]
            let u = ubuf[i % (delay + 1)]; // e[i-delay], written `delay` iterations ago
            v = a * v + (1.0 - a) * k * u;
            yy += dt * v;
            let e = r[i] as f64 - yy;
            ubuf[(i + delay) % (delay + 1)] = e;
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let nz = ((seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32 * noise_amp;
            y[i] = yy as f32 + nz;
        }
        (r, y, d2)
    }

    pub(crate) fn make_log(
        fs: f64,
        r: Vec<f32>,
        y: Vec<f32>,
        d2: Vec<f32>,
        with_segments: bool,
    ) -> FlightLog {
        let n = r.len();
        let t: Vec<f32> = (0..n).map(|i| (i as f64 / fs) as f32).collect();
        let z = vec![0f32; n];
        let mut axes: [AxisSeries; 3] = std::array::from_fn(|_| AxisSeries {
            setpoint: z.clone(),
            gyro_filt: z.clone(),
            ..Default::default()
        });
        axes[0].setpoint = r;
        axes[0].gyro_filt = y;
        let d1: Vec<f32> = vec![0.0; n];
        let chirp = with_segments.then(|| ChirpInfo {
            config: Some(ChirpConfig::default()),
            segments: vec![ChirpSegment {
                axis: Axis::Roll,
                i0: 0,
                i1: n,
                t0_s: 0.0,
                t1_s: t[n - 1],
                f_start_hz: 0.2,
                f_end_hz: 600.0,
                source: ChirpGate::Debug,
                angle_mode: false,
            }],
            debug_is_chirp: true,
        });
        FlightLog {
            id: LogId("chirp".into()),
            firmware: Firmware::Betaflight {
                version: "2026.6.1".into(),
                api: (1, 47),
            },
            fs_hz: fs,
            t,
            axes,
            motors: vec![vec![0.4; n]; 4],
            throttle: vec![0.4; n],
            erpm: None,
            gaps: vec![],
            meta: Default::default(),
            tune_at_log: Tune::Unknown,
            gyro_hr: vec![],
            debug: vec![z.clone(), d1, d2],
            chirp,
            flight_mode_flags: vec![],
        }
    }

    pub const K: f64 = std::f64::consts::TAU * 40.0;
    pub const TAU: f64 = 1.0 / (std::f64::consts::TAU * 400.0);
    pub const TD: f64 = 0.002;
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn synthetic_plant_metrics_match_analytic() {
        let fs = 4000.0;
        let (r, y, d2) = simulate(fs, 20.0, 3, K, TAU, TD, 0.0);
        let log = make_log(fs, r, y, d2, true);
        let fr = frequency_response(&log, Axis::Roll, &ChirpOpts::default()).expect("response");
        assert_eq!(fr.n_sweeps, 3);
        assert!(fr.n_windows >= 100, "{}", fr.n_windows);
        let m = &fr.metrics;
        assert!(m.coherence_mean >= 0.98, "coherence {}", m.coherence_mean);
        // analytic crossover / PM
        let mut fx = 1.0f64;
        while l_analytic(fx, K, TAU, TD).0 > 1.0 {
            fx += 0.01;
        }
        let (_, ph) = l_analytic(fx, K, TAU, TD);
        let pm = 180.0 + ph;
        assert!(
            ((m.crossover_hz as f64 - fx) / fx).abs() < 0.02,
            "crossover {} vs {fx}",
            m.crossover_hz
        );
        assert!(
            (m.phase_margin_deg as f64 - pm).abs() < 2.0,
            "PM {} vs {pm}",
            m.phase_margin_deg
        );
        // analytic −3 dB bandwidth and resonant peak of H = L/(1+L)
        let mut fb = 1.0f64;
        let mut mr = f64::NEG_INFINITY;
        let mut f = 1.0f64;
        while f < 500.0 {
            let (mag, ph) = l_analytic(f, K, TAU, TD);
            let (hm, _) = h_from_l(mag, ph);
            let db = 20.0 * hm.log10();
            mr = mr.max(db);
            f += 0.05;
        }
        while 20.0
            * h_from_l(l_analytic(fb, K, TAU, TD).0, l_analytic(fb, K, TAU, TD).1)
                .0
                .log10()
            > -3.0
        {
            fb += 0.01;
        }
        assert!(
            ((m.bandwidth_hz as f64 - fb) / fb).abs() < 0.03,
            "bandwidth {} vs {fb}",
            m.bandwidth_hz
        );
        assert!(
            (m.resonant_peak_db as f64 - mr).abs() < 0.3,
            "Mr {} vs {mr}",
            m.resonant_peak_db
        );
        // recovered open loop vs analytic over 2–150 Hz
        for (k, f) in fr.f_hz.iter().enumerate() {
            if *f < 2.0 || *f > 150.0 {
                continue;
            }
            let (mag, ph) = l_analytic(*f as f64, K, TAU, TD);
            assert!(
                (fr.l_mag_db[k] as f64 - 20.0 * mag.log10()).abs() < 1.0,
                "f={f} |L| {} vs {}",
                fr.l_mag_db[k],
                20.0 * mag.log10()
            );
            let dphi = ((fr.l_phase_deg[k] as f64 - ph + 180.0).rem_euclid(360.0)) - 180.0;
            assert!(dphi.abs() < 5.0, "f={f} ∠L {} vs {ph}", fr.l_phase_deg[k]);
        }
        // loop delay: the pure delay plus the phase slope of the first-order lag and integrator
        let slope_ref = {
            // unwrap the reference phase over 20–140 Hz before fitting the slope
            let mut prev = l_analytic(20.0, K, TAU, TD).1;
            let mut acc = 0.0;
            let mut pts: Vec<(f64, f64)> = vec![(20.0, prev)];
            let mut f = 21.0;
            while f <= 140.0 {
                let p = l_analytic(f, K, TAU, TD).1;
                let mut d = p - prev;
                while d > 180.0 {
                    d -= 360.0;
                    acc -= 360.0;
                }
                while d < -180.0 {
                    d += 360.0;
                    acc += 360.0;
                }
                prev = p;
                pts.push((f, p + acc));
                f += 1.0;
            }
            let x: Vec<f32> = pts.iter().map(|p| p.0 as f32).collect();
            let y: Vec<f32> = pts.iter().map(|p| p.1 as f32).collect();
            -(lsq_slope(&x, &y) as f64) / 360.0 * 1000.0
        };
        assert!(
            (m.loop_delay_ms as f64 - slope_ref).abs() < 0.15,
            "delay {} vs {slope_ref}",
            m.loop_delay_ms
        );
        assert!(m.targets.len() == 3 && m.targets[1].pm_deg == 60.0);
        assert!(
            m.targets[1].gain_to_target.is_finite() && m.targets[1].gain_for_sens_limit.is_finite()
        );
        assert!(m.step_overshoot.is_finite() && m.step_rise_ms.is_finite());
    }

    #[test]
    fn noise_lowers_coherence_and_masks_open_loop() {
        let fs = 4000.0;
        let (r, y, d2) = simulate(fs, 20.0, 2, K, TAU, TD, 150.0);
        let log = make_log(fs, r, y, d2, true);
        let fr = frequency_response(&log, Axis::Roll, &ChirpOpts::default()).unwrap();
        let hi: Vec<usize> = fr
            .f_hz
            .iter()
            .enumerate()
            .filter(|(_, f)| **f > 300.0 && **f < 500.0)
            .map(|(k, _)| k)
            .collect();
        let mean_coh = hi.iter().map(|k| fr.coherence[*k]).sum::<f32>() / hi.len() as f32;
        assert!(mean_coh < 0.9, "{mean_coh}");
        assert!(hi.iter().any(|k| fr.l_mag_db[*k].is_nan()));
        assert!(fr.metrics.coherence_mean > 0.5);
    }

    #[test]
    fn no_segments_gives_none() {
        let fs = 4000.0;
        let (r, y, d2) = simulate(fs, 2.0, 1, K, TAU, TD, 0.0);
        let log = make_log(fs, r, y, d2, false);
        assert!(frequency_response(&log, Axis::Roll, &ChirpOpts::default()).is_none());
        let mut log2 = make_log(fs, vec![0.0; 100], vec![0.0; 100], vec![0.0; 100], true);
        log2.chirp.as_mut().unwrap().segments[0].i1 = 100;
        assert!(frequency_response(&log2, Axis::Roll, &ChirpOpts::default()).is_none());
    }

    #[test]
    fn step_metrics_basic() {
        let t: Vec<f32> = (0..200).map(|i| i as f32 * 0.5).collect();
        let s: Vec<f32> = t.iter().map(|t| 1.0 - (-t / 5.0).exp()).collect();
        let (o, r, st) = step_metrics(&t, &s);
        assert!((o - 1.0).abs() < 0.02, "{o}");
        assert!((r - 11.0).abs() < 1.0, "{r}"); // 10→90 % of a 5 ms time constant ≈ 11 ms
        assert!(st < 20.0 && st > 10.0, "{st}");
    }
}

#[cfg(test)]
mod diag {
    use super::tests_support::*;
    use super::*;
    #[test]
    #[ignore]
    fn print_estimator_vs_reference() {
        let fs = 4000.0;
        let (r, y, d2) = simulate(fs, 20.0, 3, K, TAU, TD, 0.0);
        let log = make_log(fs, r, y, d2, true);
        let fr = frequency_response(&log, Axis::Roll, &ChirpOpts::default()).unwrap();
        for f in [
            2.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 80.0, 120.0, 150.0, 300.0,
        ] {
            let k = fr.f_hz.iter().position(|x| *x >= f).unwrap();
            let (lm, lp) = l_analytic(fr.f_hz[k] as f64, K, TAU, TD);
            let (hm, hp) = h_from_l(lm, lp);
            println!("f={:7.2} coh={:.3} |H| est {:+.2} ref {:+.2} dB  ∠H est {:+.1} ref {:+.1} | |L| est {:+.2} ref {:+.2} dB ∠L est {:+.1} ref {:+.1}",
                fr.f_hz[k], fr.coherence[k], fr.h_mag_db[k], 20.0 * hm.log10(), fr.h_phase_deg[k], hp, fr.l_mag_db[k], 20.0 * lm.log10(), fr.l_phase_deg[k], lp);
        }
        println!(
            "windows {} crossover {} PM {}",
            fr.n_windows, fr.metrics.crossover_hz, fr.metrics.phase_margin_deg
        );
    }
}
