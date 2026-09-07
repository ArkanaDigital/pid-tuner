//! ArduPilot Copter heuristics. Every threshold from ArduPilot cites its
//! source; ours are marked "ours". Parameter limits come from the generated
//! `ap_param_meta` table (apm.pdef.xml) and every recommendation is clamped.

use crate::ap_param_meta::{lookup, ParamMeta};
use domain::ap_consts::*;
use domain::*;
use uuid::Uuid;

/// ours: minimum predicted attenuation at the peak to bother the pilot with a notch change.
const MIN_NOTCH_GAIN_DB: f32 = 6.0;
/// ours: step-response targets (same as the Betaflight rules).
const OVERSHOOT_HI: f32 = 1.20;
const OVERSHOOT_OK: f32 = 1.05;
const SLOW_LATENCY_MS: f32 = 60.0;
const SS_LOW: f32 = 0.93;
/// ours: relative gain steps.
const D_STEP: f32 = 0.15;
const P_STEP: f32 = 0.10;
const I_STEP: f32 = 0.15;

fn meta_or_default(name: &str) -> ParamMeta {
    lookup(name).copied().unwrap_or(ParamMeta { name: "", min: None, max: None, increment: None, reboot_required: false, units: "", integer: false })
}

fn clamp(name: &str, v: f32) -> f32 {
    let m = meta_or_default(name);
    let mut x = v;
    if let Some(lo) = m.min {
        x = x.max(lo);
    }
    if let Some(hi) = m.max {
        x = x.min(hi);
    }
    if let Some(inc) = m.increment {
        if inc > 0.0 {
            x = (x / inc).round() * inc;
        }
    }
    if m.integer {
        x = x.round();
    }
    x
}

fn requires_reboot(name: &str) -> bool {
    // INS_HNTCH_ENABLE gates the whole HNTCH parameter group: the sub-params
    // only exist after a reboot (AP_InertialSensor.cpp, AP_SUBGROUPVARPTR).
    name.ends_with("_ENABLE") && name.starts_with("INS_HNTC") || meta_or_default(name).reboot_required
}

fn rec(name: &str, old: f32, new: f32, reason: impl Into<String>, evidence: Vec<EvidenceRef>, confidence: Confidence) -> Recommendation {
    let new = clamp(name, new);
    Recommendation {
        id: Uuid::new_v4(),
        param: ParamRef::Ap(name.to_string()),
        old: ParamValue::F32(old),
        new: ParamValue::F32(new),
        reason: reason.into(),
        evidence,
        confidence,
        requires_reboot: requires_reboot(name),
        accepted: true,
    }
}

fn axis_suffix(a: Axis) -> &'static str {
    match a {
        Axis::Roll => "RLL",
        Axis::Pitch => "PIT",
        Axis::Yaw => "YAW",
    }
}

/// Filter recommendations (Flight A: hover + batch-sampler spectra).
pub fn filters(t: &ApTune, b: &AnalysisBundle, hover_thr: Option<f32>) -> Vec<Recommendation> {
    let mut out = Vec::new();
    let g = |n: &str| t.get(n);

    // --- Harmonic notch from the dominant hover peak on the pre-filter gyro ---
    let raw_peaks: Vec<&NoisePeak> = b.peaks.iter().filter(|p| p.kind == SpectrumKind::GyroRaw && p.f_hz >= 40.0 && p.f_hz <= 400.0).collect();
    if let Some(peak) = raw_peaks.iter().max_by(|a, c| a.prominence_db.partial_cmp(&c.prominence_db).unwrap()) {
        let enabled = g("INS_HNTCH_ENABLE").unwrap_or(0.0) >= 0.5;
        let cur_freq = g("INS_HNTCH_FREQ").unwrap_or(0.0);
        // Does the logged post-filter spectrum still show the peak?
        let filt_peak = b.peaks.iter().find(|p| p.kind == SpectrumKind::GyroFilt && p.axis == peak.axis && (p.f_hz - peak.f_hz).abs() < 10.0);
        let survives = filt_peak.map(|p| p.prominence_db >= MIN_NOTCH_GAIN_DB).unwrap_or(!enabled);
        if survives {
            let ev = vec![EvidenceRef::Peak { axis: peak.axis, f_hz: peak.f_hz, psd_db: peak.psd_db }];
            let reference = hover_thr.or_else(|| g("MOT_THST_HOVER")).unwrap_or(0.35);
            if !enabled {
                out.push(rec("INS_HNTCH_ENABLE", 0.0, 1.0, format!("Motor noise peak at {:.0} Hz ({:+.0} dB) on {} reaches the controller; enable the harmonic notch (reboot, then the INS_HNTCH_* parameters appear).", peak.f_hz, peak.prominence_db, peak.axis.name()), ev.clone(), Confidence::High));
            }
            // throttle-based tracking (MODE 1), ESC RPM (MODE 3) if telemetry is logged
            let mode = if b.quality.has_pid_terms && g("INS_HNTCH_MODE").map(|m| m as i32) == Some(3) { 3.0 } else { 1.0 };
            out.push(rec("INS_HNTCH_MODE", g("INS_HNTCH_MODE").unwrap_or(0.0), mode, "Throttle-based notch tracking (mode 1) follows motor RPM with hover throttle as reference (ArduPilot 'Throttle-based notch' setup).", ev.clone(), Confidence::Medium));
            out.push(rec("INS_HNTCH_REF", g("INS_HNTCH_REF").unwrap_or(0.0), reference, format!("Reference = hover throttle ({reference:.3}, CTUN.ThH / MOT_THST_HOVER)."), ev.clone(), Confidence::Medium));
            out.push(rec("INS_HNTCH_FREQ", cur_freq, peak.f_hz, format!("Notch centre = hover motor frequency {:.0} Hz.", peak.f_hz), ev.clone(), Confidence::High));
            out.push(rec("INS_HNTCH_BW", g("INS_HNTCH_BW").unwrap_or(0.0), peak.f_hz / 2.0, "Bandwidth = FREQ/2 (ArduPilot notch setup guidance).", ev.clone(), Confidence::Medium));
            out.push(rec("INS_HNTCH_ATT", g("INS_HNTCH_ATT").unwrap_or(0.0), 40.0, "40 dB attenuation (default recommendation).", ev.clone(), Confidence::Medium));
            out.push(rec("INS_HNTCH_HMNCS", g("INS_HNTCH_HMNCS").unwrap_or(0.0), 3.0, "1st + 2nd harmonic (bitmask 3) for a quad; reboot required.", ev, Confidence::Medium));
        }
    }

    // --- Gyro low-pass and rate-loop filter relations (AP_Quicktune.cpp: FLTD = FLTT = 0.5 × INS_GYRO_FILTER) ---
    if let Some(gf) = g("INS_GYRO_FILTER") {
        for ax in ["RLL", "PIT", "YAW"] {
            for (suffix, mul) in [("FLTD", QUIK_FLTD_MUL), ("FLTT", QUIK_FLTT_MUL)] {
                let name = format!("ATC_RAT_{ax}_{suffix}");
                if let Some(cur) = g(&name) {
                    let want = (gf * mul).round();
                    if ax == "YAW" && suffix == "FLTD" {
                        continue; // yaw D is normally 0; FLTD irrelevant
                    }
                    if (cur - want).abs() > 2.0 && cur > 0.0 {
                        out.push(rec(&name, cur, want, format!("Quicktune relation: {suffix} = INS_GYRO_FILTER × {mul} = {want:.0} Hz (currently {cur:.0})."), vec![EvidenceRef::Text { note: format!("INS_GYRO_FILTER = {gf}") }], Confidence::Low));
                    }
                }
            }
        }
    }
    dedupe(out)
}

/// PID recommendations (Flight B: step responses at loop rate + SRate).
pub fn pids(t: &ApTune, b: &AnalysisBundle, max_srate: Option<[f32; 3]>) -> Vec<Recommendation> {
    let mut out = Vec::new();
    let g = |n: &str| t.get(n);
    let osc_smax = g("QUIK_OSC_SMAX").unwrap_or(QUIK_OSC_SMAX_DEFAULT);
    let margin = g("QUIK_GAIN_MARGIN").unwrap_or(QUIK_GAIN_MARGIN_DEFAULT_PCT) / 100.0;
    let sat = b.quality.max_pid_out.unwrap_or([0.0; 3]);

    for s in &b.steps {
        if s.n_segments < 5 {
            continue;
        }
        let ax = axis_suffix(s.axis);
        let k = s.axis.index();
        let (pn, in_, dn) = (format!("ATC_RAT_{ax}_P"), format!("ATC_RAT_{ax}_I"), format!("ATC_RAT_{ax}_D"));
        let (Some(p), Some(i)) = (g(&pn), g(&in_)) else { continue };
        let d = g(&dn).unwrap_or(0.0);
        let ev = vec![EvidenceRef::Step { axis: s.axis, overshoot: s.overshoot, latency_ms: s.latency_ms }];
        let saturated = sat[k] >= 0.9;

        // Quicktune oscillation criterion: slew rate above QUIK_OSC_SMAX → back off by QUIK_GAIN_MARGIN.
        if let Some(sr) = max_srate.map(|m| m[k]) {
            if sr > osc_smax {
                let f = 1.0 - margin;
                out.push(rec(&pn, p, p * f, format!("{} rate loop oscillates: PID slew rate {:.1} > QUIK_OSC_SMAX {:.0}; Quicktune backs the gain off by {:.0} %.", s.axis.name(), sr, osc_smax, margin * 100.0), vec![EvidenceRef::Quality { field: "srate".into(), value: sr as f64 }], Confidence::High));
                out.push(rec(&in_, i, i * f, "I follows P (Quicktune keeps the P:I ratio).", vec![], Confidence::High));
                if d > 0.0 {
                    out.push(rec(&dn, d, d * f, "D backed off with P.", vec![], Confidence::High));
                }
                continue;
            }
        }
        if s.overshoot > OVERSHOOT_HI {
            if s.axis != Axis::Yaw && d > 0.0 {
                out.push(rec(&dn, d, d * (1.0 + D_STEP), format!("{} overshoots to {:.2} (target ≤ 1.10); more D damps it.", s.axis.name(), s.overshoot), ev.clone(), Confidence::Medium));
            }
            if s.overshoot > 1.35 || s.axis == Axis::Yaw {
                out.push(rec(&pn, p, p * (1.0 - P_STEP), format!("{} overshoot {:.2}: reduce P.", s.axis.name(), s.overshoot), ev.clone(), Confidence::Medium));
                out.push(rec(&in_, i, i * (1.0 - P_STEP), "I follows P (keep the P:I ratio).", vec![], Confidence::Medium));
            }
        } else if s.overshoot < OVERSHOOT_OK && s.latency_ms.is_finite() && s.latency_ms > SLOW_LATENCY_MS && !saturated {
            out.push(rec(&pn, p, p * (1.0 + P_STEP), format!("{} reaches 50 % only after {:.0} ms with no overshoot; raise P.", s.axis.name(), s.latency_ms), ev.clone(), Confidence::Medium));
            out.push(rec(&in_, i, i * (1.0 + P_STEP), "I follows P (keep the P:I ratio).", vec![], Confidence::Medium));
        }
        if s.steady_state < SS_LOW && !saturated {
            out.push(rec(&in_, i, i * (1.0 + I_STEP), format!("{} settles at {:.2}; raise I to remove the steady-state error.", s.axis.name(), s.steady_state), ev.clone(), Confidence::Medium));
        }
        if saturated {
            out.push(rec(&pn, p, p * (1.0 - P_STEP), format!("{} mixer output saturates (RATE.*Out {:.2}); only gain reductions are safe.", s.axis.name(), sat[k]), vec![EvidenceRef::Quality { field: "max_pid_out".into(), value: sat[k] as f64 }], Confidence::Medium));
        }
    }

    // AUTOTUNE failure fingerprint: D left at the floor.
    let min_d = g("AUTOTUNE_MIN_D").unwrap_or(0.001);
    for ax in ["RLL", "PIT"] {
        let dn = format!("ATC_RAT_{ax}_D");
        if let Some(d) = g(&dn) {
            if d > 0.0 && d <= min_d + 1e-9 {
                out.push(rec("AUTOTUNE_AGGR", g("AUTOTUNE_AGGR").unwrap_or(0.1), 0.075, format!("{dn} = {d} sits at AUTOTUNE_MIN_D: the last AUTOTUNE failed on this axis (noise/flex). Re-run with lower aggressiveness."), vec![EvidenceRef::Text { note: format!("{dn} == AUTOTUNE_MIN_D") }], Confidence::Medium));
            }
        }
    }
    // SMAX guard rail (Quicktune sets 50 when 0)
    for ax in ["RLL", "PIT", "YAW"] {
        let n = format!("ATC_RAT_{ax}_SMAX");
        if g(&n) == Some(0.0) {
            out.push(rec(&n, 0.0, 50.0, "Slew-rate limiter off; Quicktune's DEFAULT_SMAX = 50 protects against oscillation while tuning.", vec![], Confidence::Low));
        }
    }
    dedupe(out)
}

/// Later recommendations for the same parameter merge into the first one
/// (multiplicative steps compose) so a parameter appears once.
fn dedupe(v: Vec<Recommendation>) -> Vec<Recommendation> {
    let mut out: Vec<Recommendation> = Vec::new();
    for r in v {
        if let Some(e) = out.iter_mut().find(|e| e.param == r.param) {
            let ratio = if e.old.as_f64().abs() > 1e-12 { r.new.as_f64() / e.old.as_f64() } else { 1.0 };
            let merged = clamp(e.param.name(), (e.new.as_f64() * ratio) as f32);
            e.new = ParamValue::F32(merged);
            e.reason.push_str(" Also: ");
            e.reason.push_str(&r.reason);
            e.evidence.extend(r.evidence);
        } else {
            out.push(r);
        }
    }
    out
}

/// Max |SRate| per axis over the airborne range (Quicktune criterion input).
pub fn max_srate(log: &FlightLog, range_s: Option<(f32, f32)>) -> Option<[f32; 3]> {
    let (i0, i1) = match range_s {
        Some((a, b)) => dsp::decimate::range_indices(&log.t, a, b),
        None => (0, log.len()),
    };
    let mut out = [0f32; 3];
    for (k, ax) in log.axes.iter().enumerate() {
        let s = ax.srate.as_ref()?;
        out[k] = s[i0.min(s.len())..i1.min(s.len())].iter().fold(0f32, |m, v| m.max(v.abs()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> ApTune {
        let mut t = ApTune::default();
        for (k, v) in [("ATC_RAT_RLL_P", 0.135), ("ATC_RAT_RLL_I", 0.135), ("ATC_RAT_RLL_D", 0.0036), ("ATC_RAT_PIT_P", 0.135), ("ATC_RAT_PIT_I", 0.135), ("ATC_RAT_PIT_D", 0.0036), ("ATC_RAT_YAW_P", 0.18), ("ATC_RAT_YAW_I", 0.018), ("ATC_RAT_YAW_D", 0.0), ("INS_GYRO_FILTER", 40.0), ("ATC_RAT_RLL_FLTD", 20.0), ("ATC_RAT_RLL_FLTT", 20.0), ("ATC_RAT_PIT_FLTD", 20.0), ("ATC_RAT_PIT_FLTT", 20.0), ("ATC_RAT_YAW_FLTT", 20.0), ("INS_HNTCH_ENABLE", 0.0), ("AUTOTUNE_MIN_D", 0.001), ("ATC_RAT_RLL_SMAX", 50.0), ("ATC_RAT_PIT_SMAX", 50.0), ("ATC_RAT_YAW_SMAX", 50.0)] {
            t.params.insert(k.into(), v);
        }
        t
    }

    fn bundle(peaks: Vec<NoisePeak>, steps: Vec<StepResponse>) -> AnalysisBundle {
        AnalysisBundle { log: LogId("x".into()), quality: LogQuality { max_pid_out: Some([0.3, 0.3, 0.2]), has_pid_terms: true, ..Default::default() }, steps, spectra: vec![], spectrograms: vec![], peaks, anomalies: vec![] }
    }

    fn step(axis: Axis, overshoot: f32, latency_ms: f32, ss: f32) -> StepResponse {
        StepResponse { axis, variant: StepVariant::PtStep, t_ms: vec![], mean: vec![], p10: vec![], p90: vec![], n_segments: 40, rejected: 0, overshoot, latency_ms, settle_ms: None, steady_state: ss }
    }

    #[test]
    fn hover_peak_enables_harmonic_notch_with_reboot_flags() {
        let b = bundle(vec![NoisePeak { axis: Axis::Roll, kind: SpectrumKind::GyroRaw, f_hz: 80.0, psd_db: 5.0, prominence_db: 25.0, band: NoiseBand::Control }], vec![]);
        let r = filters(&tune(), &b, Some(0.35));
        let by = |n: &str| r.iter().find(|x| x.param.name() == n).unwrap_or_else(|| panic!("{n} missing: {:?}", r.iter().map(|x| x.param.name()).collect::<Vec<_>>()));
        assert!(by("INS_HNTCH_ENABLE").requires_reboot);
        assert!(by("INS_HNTCH_HMNCS").requires_reboot);
        assert!(!by("INS_HNTCH_FREQ").requires_reboot);
        assert_eq!(by("INS_HNTCH_FREQ").new.as_f64(), 80.0);
        assert_eq!(by("INS_HNTCH_BW").new.as_f64(), 40.0);
        assert!((by("INS_HNTCH_REF").new.as_f64() - 0.35).abs() < 1e-6);
        // every param must exist in the pdef table and be within range
        for x in &r {
            let m = lookup(x.param.name()).unwrap_or_else(|| panic!("{} not in pdef table", x.param.name()));
            if let Some(lo) = m.min { assert!(x.new.as_f64() >= lo as f64); }
            if let Some(hi) = m.max { assert!(x.new.as_f64() <= hi as f64); }
        }
    }

    #[test]
    fn notch_not_suggested_when_post_filter_is_clean() {
        let mut t = tune();
        t.params.insert("INS_HNTCH_ENABLE".into(), 1.0);
        let b = bundle(vec![
            NoisePeak { axis: Axis::Roll, kind: SpectrumKind::GyroRaw, f_hz: 80.0, psd_db: 5.0, prominence_db: 25.0, band: NoiseBand::Control },
            NoisePeak { axis: Axis::Roll, kind: SpectrumKind::GyroFilt, f_hz: 80.0, psd_db: -30.0, prominence_db: 3.0, band: NoiseBand::Control },
        ], vec![]);
        let r = filters(&t, &b, Some(0.35));
        assert!(r.iter().all(|x| !x.param.name().starts_with("INS_HNTCH_FREQ")), "{:?}", r.iter().map(|x| x.param.name()).collect::<Vec<_>>());
    }

    #[test]
    fn quicktune_oscillation_backs_off_only_that_axis() {
        let b = bundle(vec![], vec![step(Axis::Roll, 1.05, 20.0, 1.0), step(Axis::Pitch, 1.05, 20.0, 1.0)]);
        let r = pids(&tune(), &b, Some([5.0, 1.0, 1.0]));
        let rp = r.iter().find(|x| x.param.name() == "ATC_RAT_RLL_P").unwrap();
        // 0.135 × (1 − 0.60) = 0.054 → pdef increment 0.005 → 0.055
        assert!((rp.new.as_f64() - 0.055).abs() < 1e-6, "{}", rp.new);
        assert!(rp.reason.contains("QUIK_OSC_SMAX"));
        assert!(r.iter().all(|x| !x.param.name().starts_with("ATC_RAT_PIT")));
    }

    #[test]
    fn overshoot_raises_d_and_clamps_to_pdef_range() {
        let b = bundle(vec![], vec![step(Axis::Pitch, 1.30, 15.0, 1.0)]);
        let r = pids(&tune(), &b, None);
        let d = r.iter().find(|x| x.param.name() == "ATC_RAT_PIT_D").unwrap();
        assert!((d.new.as_f64() - 0.004).abs() < 1e-6, "{}", d.new); // 0.0036*1.15=0.00414 → increment 0.001 → 0.004
        let mut t = tune();
        t.params.insert("ATC_RAT_PIT_D".into(), 0.029);
        let r = pids(&t, &b, None);
        let d = r.iter().find(|x| x.param.name() == "ATC_RAT_PIT_D").unwrap();
        assert!(d.new.as_f64() <= 0.03 + 1e-6, "clamped to pdef max: {}", d.new);
    }

    #[test]
    fn autotune_min_d_fingerprint() {
        let mut t = tune();
        t.params.insert("ATC_RAT_RLL_D".into(), 0.001);
        let r = pids(&t, &bundle(vec![], vec![]), None);
        assert!(r.iter().any(|x| x.param.name() == "AUTOTUNE_AGGR" && x.reason.contains("AUTOTUNE_MIN_D")));
    }

    #[test]
    fn saturation_blocks_gain_increase() {
        let mut b = bundle(vec![], vec![step(Axis::Roll, 1.0, 80.0, 0.9)]);
        b.quality.max_pid_out = Some([0.95, 0.2, 0.2]);
        let r = pids(&tune(), &b, None);
        let p = r.iter().find(|x| x.param.name() == "ATC_RAT_RLL_P").unwrap();
        assert!(p.new.as_f64() < 0.135, "{}", p.new);
        assert!(r.iter().all(|x| x.param.name() != "ATC_RAT_RLL_I" || x.new.as_f64() <= 0.135));
    }
}
