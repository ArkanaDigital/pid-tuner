//! Betaflight PID/FF recommendations from CHIRP frequency responses.
//!
//! Follows the gain logic of betaflight-configurator's Autotune tab
//! (`spectral_analysis.js::computeGainScales`, GPL-3.0-or-later) with a
//! stricter safety envelope: recommendations only when coherence is good,
//! per-iteration scale clamped to [0.75, 1.25] (Configurator: [0.5, 2]),
//! D untouched, and **never** a filter change derived from the sweep
//! (Configurator issue #5258).

use crate::bf::{clamp_u16, clamp_u8, rec};
use domain::*;

/// ours: gate before any recommendation
pub const MIN_COHERENCE: f32 = 0.6;
pub const MIN_WINDOWS: usize = 8;
/// ours: per-iteration gain change envelope
const SCALE_LO: f32 = 0.75;
const SCALE_HI: f32 = 1.25;
/// Configurator constants
const TARGET_PM_DEG: f32 = 60.0;
const MAX_SENSITIVITY_PEAK: f32 = 2.0;
const GAIN_SCAN_STEP: f32 = 0.01;
/// ours: ignore changes smaller than this
const MIN_CHANGE: f32 = 0.03;

#[derive(Debug, Clone, Copy)]
pub struct AxisScales {
    pub axis: Axis,
    pub pi: f32,
    pub i: f32,
    pub ff: f32,
    pub coherence: f32,
    pub bandwidth_hz: f32,
    pub phase_margin_deg: f32,
    pub sens_backoff: bool,
}

/// Gain scales for one axis, `None` when the measurement is not good enough.
pub fn scales_for(fr: &FrequencyResponse) -> Result<AxisScales, String> {
    let m = &fr.metrics;
    if fr.angle_mode {
        return Err(format!("{}: sweep flown in ANGLE/HORIZON mode — the attitude loop is inside the measurement; fly the chirp in ACRO.", fr.axis.name()));
    }
    if fr.n_windows < MIN_WINDOWS {
        return Err(format!(
            "{}: only {} Welch windows (need ≥ {MIN_WINDOWS}); fly longer or more sweeps.",
            fr.axis.name(),
            fr.n_windows
        ));
    }
    if m.coherence_mean.is_nan() || m.coherence_mean < MIN_COHERENCE {
        return Err(format!("{}: mean coherence {:.2} (need ≥ {MIN_COHERENCE}); the gyro did not follow the sweep — calmer air, ACRO, higher amplitude.", fr.axis.name(), m.coherence_mean));
    }
    let t60 = m
        .targets
        .iter()
        .find(|t| (t.pm_deg - TARGET_PM_DEG).abs() < 0.1);
    let gain_to_target = t60.map(|t| t.gain_to_target).unwrap_or(f32::NAN);
    if !(m.crossover_hz.is_finite()
        && m.phase_margin_deg.is_finite()
        && m.resonant_peak_db.is_finite())
    {
        return Err(format!("{}: no usable crossover/phase margin (open loop not coherent between 2 Hz and the crossover).", fr.axis.name()));
    }
    let gain_for_margin = if gain_to_target.is_finite() {
        gain_to_target
    } else {
        1.0
    };
    let (backoff, ff_backoff) = if m.resonant_peak_db > 6.0 {
        (0.75, 0.8)
    } else if m.resonant_peak_db > 3.0 {
        (0.9, 1.0)
    } else {
        (1.0, 1.0)
    };
    let requested = gain_for_margin * backoff;
    let mut pi = requested.clamp(SCALE_LO, SCALE_HI);
    // Sensitivity-peak limit: use the measured gain-for-limit from the response when it binds.
    let mut sens_backoff = false;
    if let Some(t) = t60 {
        if t.gain_for_sens_limit.is_finite() && t.gain_for_sens_limit < pi {
            pi = t.gain_for_sens_limit.max(SCALE_LO);
            sens_backoff = true;
        }
    }
    let _ = GAIN_SCAN_STEP;
    let robustness = if requested.clamp(SCALE_LO, SCALE_HI) > 0.0 {
        pi / requested.clamp(SCALE_LO, SCALE_HI)
    } else {
        1.0
    };
    let i = if m.low_freq_err_db.is_finite() {
        (10f32.powf(-m.low_freq_err_db / 20.0)).clamp(0.85, 1.2)
    } else {
        1.0
    };
    let ff = (gain_for_margin * ff_backoff * robustness).clamp(0.8, 1.2);
    let _ = MAX_SENSITIVITY_PEAK;
    Ok(AxisScales {
        axis: fr.axis,
        pi,
        i,
        ff,
        coherence: m.coherence_mean,
        bandwidth_hz: m.bandwidth_hz,
        phase_margin_deg: m.phase_margin_deg,
        sens_backoff,
    })
}

fn changed(scale: f32) -> bool {
    (scale - 1.0).abs() >= MIN_CHANGE
}

fn conf(s: &AxisScales, scale: f32) -> Confidence {
    if s.coherence >= 0.8 && (scale - 1.0).abs() <= 0.15 {
        Confidence::High
    } else {
        Confidence::Medium
    }
}

/// Recommendations from the chirp responses. Returns the recs and which axes
/// were covered (so the step-response rules skip them).
pub fn rules(t: &BfTune, b: &AnalysisBundle) -> (Vec<Recommendation>, [bool; 3]) {
    let mut out = Vec::new();
    let mut covered = [false; 3];
    if b.freq_resp.is_empty() {
        return (out, covered);
    }
    let mut scales: Vec<AxisScales> = Vec::new();
    for fr in &b.freq_resp {
        match scales_for(fr) {
            Ok(s) => {
                covered[fr.axis.index()] = true;
                scales.push(s);
            }
            Err(why) => out.push(Recommendation {
                id: uuid::Uuid::new_v4(),
                param: ParamRef::Bf(format!("chirp_{}", fr.axis.name().to_lowercase())),
                old: ParamValue::Bool(false),
                new: ParamValue::Bool(false),
                reason: format!("No PID change from the CHIRP sweep: {why}"),
                evidence: vec![EvidenceRef::FreqResp {
                    axis: fr.axis,
                    bandwidth_hz: fr.metrics.bandwidth_hz,
                    phase_margin_deg: fr.metrics.phase_margin_deg,
                    coherence: fr.metrics.coherence_mean,
                }],
                confidence: Confidence::Low,
                requires_reboot: false,
                accepted: false,
            }),
        }
    }
    if scales.is_empty() {
        return (out, covered);
    }
    let ev = |s: &AxisScales| {
        vec![EvidenceRef::FreqResp {
            axis: s.axis,
            bandwidth_hz: s.bandwidth_hz,
            phase_margin_deg: s.phase_margin_deg,
            coherence: s.coherence,
        }]
    };
    let why = |s: &AxisScales, what: &str, scale: f32| {
        format!(
            "{} CHIRP response: bandwidth {:.0} Hz, phase margin {:.0}° (target {TARGET_PM_DEG:.0}°), coherence {:.2}{} → {what} ×{scale:.2}. D is left unchanged (a sweep cannot separate P from D).",
            s.axis.name(),
            s.bandwidth_hz,
            s.phase_margin_deg,
            s.coherence,
            if s.sens_backoff { ", limited by the sensitivity peak (6 dB)" } else { "" }
        )
    };

    if t.simplified.pids_mode == 0 {
        for s in &scales {
            let k = s.axis.index();
            let ax = ["roll", "pitch", "yaw"][k];
            let pid = t.pids[k];
            if changed(s.pi) {
                out.push(rec(
                    &format!("p_{ax}"),
                    ParamValue::U8(pid.p),
                    ParamValue::U8(clamp_u8((pid.p as f32 * s.pi).round() as i32, 1, 250)),
                    why(s, "P", s.pi),
                    ev(s),
                    conf(s, s.pi),
                ));
            }
            let i_scale = s.pi * s.i;
            if changed(i_scale) {
                out.push(rec(
                    &format!("i_{ax}"),
                    ParamValue::U8(pid.i),
                    ParamValue::U8(clamp_u8((pid.i as f32 * i_scale).round() as i32, 1, 250)),
                    why(s, "I (with P, plus low-frequency error)", i_scale),
                    ev(s),
                    conf(s, i_scale),
                ));
            }
            if pid.ff > 0 && changed(s.ff) {
                out.push(rec(
                    &format!("f_{ax}"),
                    ParamValue::U16(pid.ff),
                    ParamValue::U16(clamp_u16((pid.ff as f32 * s.ff).round() as i32, 0, 1000)),
                    why(s, "FF", s.ff),
                    ev(s),
                    conf(s, s.ff),
                ));
            }
        }
    } else {
        // Simplified sliders ON: scale the sliders instead of the raw values, never touch D or filters.
        let sm = &t.simplified;
        let base = scales
            .iter()
            .find(|s| s.axis == Axis::Roll)
            .or_else(|| scales.iter().find(|s| s.axis == Axis::Pitch));
        if let Some(b0) = base {
            let sl = |cur: u8, scale: f32| {
                ParamValue::U8(clamp_u8((cur as f32 * scale).round() as i32, 25, 250))
            };
            if changed(b0.pi) {
                out.push(rec(
                    "simplified_pi_gain",
                    ParamValue::U8(sm.pi_gain),
                    sl(sm.pi_gain, b0.pi),
                    why(b0, "PI slider", b0.pi),
                    ev(b0),
                    conf(b0, b0.pi),
                ));
            }
            if changed(b0.i) {
                out.push(rec(
                    "simplified_i_gain",
                    ParamValue::U8(sm.i_gain),
                    sl(sm.i_gain, b0.i),
                    why(b0, "I slider", b0.i),
                    ev(b0),
                    conf(b0, b0.i),
                ));
            }
            if changed(b0.ff) {
                out.push(rec(
                    "simplified_feedforward_gain",
                    ParamValue::U8(sm.feedforward_gain),
                    sl(sm.feedforward_gain, b0.ff),
                    why(b0, "FF slider", b0.ff),
                    ev(b0),
                    conf(b0, b0.ff),
                ));
            }
            if let (Some(p), true) = (
                scales.iter().find(|s| s.axis == Axis::Pitch),
                b0.axis == Axis::Roll,
            ) {
                let ratio = p.pi / b0.pi;
                if changed(ratio) {
                    out.push(rec(
                        "simplified_pitch_pi_gain",
                        ParamValue::U8(sm.pitch_pi_gain),
                        sl(sm.pitch_pi_gain, ratio),
                        why(p, "pitch PI slider relative to roll", ratio),
                        ev(p),
                        conf(p, ratio),
                    ));
                }
            }
        }
    }
    (out, covered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(
        axis: Axis,
        coh: f32,
        windows: usize,
        pm: f32,
        gain60: f32,
        sens_gain: f32,
        mr: f32,
        lf: f32,
    ) -> FrequencyResponse {
        FrequencyResponse {
            axis,
            angle_mode: false,
            f_hz: vec![],
            h_mag_db: vec![],
            h_phase_deg: vec![],
            coherence: vec![],
            l_mag_db: vec![],
            l_phase_deg: vec![],
            s_mag_db: vec![],
            step_t_ms: vec![],
            step: vec![],
            fs_hz: 2000.0,
            segment_size: 1024,
            n_windows: windows,
            n_sweeps: 2,
            sweep_seconds: 20.0,
            metrics: FrMetrics {
                bandwidth_hz: 40.0,
                crossover_hz: 30.0,
                phase_margin_deg: pm,
                max_phase_margin_deg: 80.0,
                resonant_peak_db: mr,
                resonant_peak_hz: 25.0,
                loop_delay_ms: 3.0,
                low_freq_err_db: lf,
                coherence_mean: coh,
                noise_floor_hz: 150.0,
                sens_peak_db: 3.0,
                sens_peak_hz: 50.0,
                step_overshoot: 1.05,
                step_rise_ms: 10.0,
                step_settle_ms: 40.0,
                targets: vec![
                    FrTarget {
                        pm_deg: 50.0,
                        crossover_hz: 45.0,
                        gain_to_target: gain60 * 1.2,
                        gain_for_sens_limit: sens_gain,
                    },
                    FrTarget {
                        pm_deg: 60.0,
                        crossover_hz: 35.0,
                        gain_to_target: gain60,
                        gain_for_sens_limit: sens_gain,
                    },
                    FrTarget {
                        pm_deg: 72.5,
                        crossover_hz: 20.0,
                        gain_to_target: gain60 * 0.7,
                        gain_for_sens_limit: sens_gain,
                    },
                ],
            },
        }
    }

    fn tune() -> BfTune {
        let mut t = BfTune::default();
        t.simplified.pids_mode = 0;
        t.pids = [
            BfAxisPid {
                p: 45,
                i: 80,
                d: 30,
                ff: 120,
                d_max: 40,
            },
            BfAxisPid {
                p: 47,
                i: 84,
                d: 34,
                ff: 125,
                d_max: 46,
            },
            BfAxisPid {
                p: 45,
                i: 80,
                d: 0,
                ff: 120,
                d_max: 0,
            },
        ];
        t
    }

    fn bundle(frs: Vec<FrequencyResponse>) -> AnalysisBundle {
        AnalysisBundle {
            log: LogId("x".into()),
            quality: LogQuality::default(),
            steps: vec![],
            spectra: vec![],
            spectrograms: vec![],
            peaks: vec![],
            anomalies: vec![],
            freq_resp: frs,
        }
    }

    #[test]
    fn gate_blocks_low_coherence_few_windows_and_angle_mode() {
        let t = tune();
        let (r, cov) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.5, 50, 55.0, 1.3, 2.0, 1.0, 0.0)]),
        );
        assert!(!cov[0] && r.len() == 1 && r[0].reason.contains("coherence"));
        let (r, cov) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.9, 5, 55.0, 1.3, 2.0, 1.0, 0.0)]),
        );
        assert!(!cov[0] && r[0].reason.contains("windows"));
        let mut a = fr(Axis::Pitch, 0.9, 50, 55.0, 1.3, 2.0, 1.0, 0.0);
        a.angle_mode = true;
        let (r, cov) = rules(&t, &bundle(vec![a]));
        assert!(!cov[1] && r[0].reason.contains("ANGLE"));
        assert!(r.iter().all(|x| !x.accepted));
    }

    #[test]
    fn gain_to_target_is_clamped_and_backed_off_by_resonance_and_sensitivity() {
        let t = tune();
        // wants ×1.8 → clamped to 1.25
        let (r, cov) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.9, 50, 40.0, 1.8, 2.0, 1.0, 0.0)]),
        );
        assert!(cov[0]);
        let p = r.iter().find(|x| x.param.name() == "p_roll").unwrap();
        assert_eq!(p.new, ParamValue::U8((45.0f32 * 1.25).round() as u8));
        // resonance 7 dB → ×0.75 backoff on a ×1.2 request = 0.9
        let (r, _) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.9, 50, 40.0, 1.2, 2.0, 7.0, 0.0)]),
        );
        let p = r.iter().find(|x| x.param.name() == "p_roll").unwrap();
        assert_eq!(p.new, ParamValue::U8((45.0f32 * 0.9).round() as u8));
        // sensitivity limit binds at 1.05 although the target asks 1.2
        let (r, _) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.9, 50, 40.0, 1.2, 1.05, 1.0, 0.0)]),
        );
        let p = r.iter().find(|x| x.param.name() == "p_roll").unwrap();
        assert_eq!(p.new, ParamValue::U8((45.0f32 * 1.05).round() as u8));
        assert!(p.reason.contains("sensitivity"));
        // never below 0.75
        let (r, _) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.9, 50, 40.0, 0.3, 2.0, 1.0, 0.0)]),
        );
        let p = r.iter().find(|x| x.param.name() == "p_roll").unwrap();
        assert_eq!(p.new, ParamValue::U8((45.0f32 * 0.75).round() as u8));
    }

    #[test]
    fn small_changes_are_skipped_and_d_is_never_touched() {
        let t = tune();
        let (r, cov) = rules(
            &t,
            &bundle(vec![fr(Axis::Roll, 0.95, 50, 60.0, 1.01, 2.0, 0.5, 0.0)]),
        );
        assert!(cov[0]);
        assert!(r.is_empty(), "{r:?}");
        let (r, _) = rules(
            &t,
            &bundle(vec![fr(Axis::Pitch, 0.95, 50, 45.0, 1.15, 2.0, 0.5, -2.0)]),
        );
        let names: Vec<&str> = r.iter().map(|x| x.param.name()).collect();
        assert!(
            names.contains(&"p_pitch") && names.contains(&"i_pitch") && names.contains(&"f_pitch"),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.starts_with("d_")));
        // I includes the low-frequency error: −2 dB → ×1.26 on top of P ×1.15
        let i = r.iter().find(|x| x.param.name() == "i_pitch").unwrap();
        assert_eq!(i.new, ParamValue::U8((84.0f32 * 1.15 * 1.2).round() as u8));
        // lf scale clamped to 1.2
    }

    #[test]
    fn slider_path_emits_only_slider_names_and_no_filters() {
        let mut t = tune();
        t.simplified.pids_mode = 2;
        t.simplified.pi_gain = 100;
        t.simplified.i_gain = 100;
        t.simplified.feedforward_gain = 100;
        t.simplified.pitch_pi_gain = 100;
        let (r, _) = rules(
            &t,
            &bundle(vec![
                fr(Axis::Roll, 0.9, 50, 45.0, 1.15, 2.0, 0.5, 0.0),
                fr(Axis::Pitch, 0.9, 50, 45.0, 1.25, 2.0, 0.5, 0.0),
            ]),
        );
        let names: Vec<&str> = r.iter().map(|x| x.param.name()).collect();
        assert!(
            names.contains(&"simplified_pi_gain")
                && names.contains(&"simplified_feedforward_gain")
                && names.contains(&"simplified_pitch_pi_gain"),
            "{names:?}"
        );
        assert!(
            names.iter().all(|n| n.starts_with("simplified_")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("filter")
                || n.contains("lpf")
                || n.contains("notch")
                || n.contains("dterm")
                || *n == "simplified_d_gain"
                || *n == "simplified_d_max_gain"
                || n.contains("pids_mode")),
            "{names:?}"
        );
        let pp = r
            .iter()
            .find(|x| x.param.name() == "simplified_pitch_pi_gain")
            .unwrap();
        assert_eq!(
            pp.new,
            ParamValue::U8((100.0f32 * 1.25 / 1.15).round() as u8)
        );
    }
}
