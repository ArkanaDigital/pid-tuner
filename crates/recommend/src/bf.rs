//! Betaflight heuristics. Sources: Betaflight PID Tuning Guide, 4.3 Tuning
//! Notes, Oscar Liang's blackbox filter/PID guide, PIDtoolbox conventions.

use domain::*;
use uuid::Uuid;

pub(crate) fn rec(
    name: &str,
    old: ParamValue,
    new: ParamValue,
    reason: impl Into<String>,
    evidence: Vec<EvidenceRef>,
    confidence: Confidence,
) -> Recommendation {
    Recommendation {
        id: Uuid::new_v4(),
        param: ParamRef::Bf(name.to_string()),
        old,
        new,
        reason: reason.into(),
        evidence,
        confidence,
        requires_reboot: false,
        accepted: true,
    }
}

pub(crate) fn clamp_u8(v: i32, lo: u8, hi: u8) -> u8 {
    v.clamp(lo as i32, hi as i32) as u8
}
pub(crate) fn clamp_u16(v: i32, lo: u16, hi: u16) -> u16 {
    v.clamp(lo as i32, hi as i32) as u16
}

fn peak_ev(p: &NoisePeak) -> EvidenceRef {
    EvidenceRef::Peak { axis: p.axis, f_hz: p.f_hz, psd_db: p.psd_db }
}

/// Simplified-tuning sliders must be OFF before raw values are written, else the
/// FC recomputes and overwrites them.
fn simplified_off(t: &BfTune, out: &mut Vec<Recommendation>, need_pids: bool, need_filters: bool) {
    if need_pids && t.simplified.pids_mode != 0 {
        out.push(rec(
            "simplified_pids_mode",
            ParamValue::Enum(t.simplified.pids_mode),
            ParamValue::Enum(0),
            "Simplified PID sliders are ON; they must be OFF so explicit P/I/D/FF values are kept.",
            vec![EvidenceRef::Text { note: format!("simplified_pids_mode = {}", t.simplified.pids_mode) }],
            Confidence::High,
        ));
    }
    if need_filters && t.simplified.gyro_filter {
        out.push(rec(
            "simplified_gyro_filter",
            ParamValue::Bool(true),
            ParamValue::Bool(false),
            "Gyro filter slider is ON; disable it so explicit gyro filter cutoffs are kept.",
            vec![],
            Confidence::High,
        ));
    }
    if need_filters && t.simplified.dterm_filter {
        out.push(rec(
            "simplified_dterm_filter",
            ParamValue::Bool(true),
            ParamValue::Bool(false),
            "D-term filter slider is ON; disable it so explicit D-term filter cutoffs are kept.",
            vec![],
            Confidence::High,
        ));
    }
}

/// Filter recommendations from the noise spectra (Flight A: hover + wobble).
pub fn filters(t: &BfTune, b: &AnalysisBundle) -> Vec<Recommendation> {
    let mut out = Vec::new();
    let f = &t.filters;
    let raw_peaks: Vec<&NoisePeak> = b.peaks.iter().filter(|p| p.kind == SpectrumKind::GyroRaw).collect();
    let filt_peaks: Vec<&NoisePeak> = b.peaks.iter().filter(|p| p.kind == SpectrumKind::GyroFilt).collect();
    let dterm_peaks: Vec<&NoisePeak> = b.peaks.iter().filter(|p| p.kind == SpectrumKind::DTerm).collect();

    let mut changes: Vec<Recommendation> = Vec::new();

    // --- Frame resonance (100–250 Hz) surviving in the filtered gyro → dyn notch ---
    let frame_surviving: Vec<&&NoisePeak> = filt_peaks
        .iter()
        .filter(|p| p.band == NoiseBand::Frame && p.prominence_db >= 10.0)
        .collect();
    if let Some(p) = frame_surviving.iter().max_by(|a, b| a.prominence_db.partial_cmp(&b.prominence_db).unwrap()) {
        let ev = vec![peak_ev(p)];
        if f.dyn_notch_count < 3 {
            changes.push(rec(
                "dyn_notch_count",
                ParamValue::U8(f.dyn_notch_count),
                ParamValue::U8(f.dyn_notch_count + 1),
                format!(
                    "Frame resonance at {:.0} Hz ({:+.0} dB above floor) is still present after filtering; an extra dynamic notch will track it.",
                    p.f_hz, p.prominence_db
                ),
                ev.clone(),
                Confidence::Medium,
            ));
        }
        if p.f_hz < f.dyn_notch_min_hz as f32 {
            changes.push(rec(
                "dyn_notch_min_hz",
                ParamValue::U16(f.dyn_notch_min_hz),
                ParamValue::U16(clamp_u16((p.f_hz * 0.8) as i32, 20, 250)),
                format!("Resonance at {:.0} Hz is below dyn_notch_min_hz ({}), so the dynamic notch cannot reach it.", p.f_hz, f.dyn_notch_min_hz),
                ev.clone(),
                Confidence::High,
            ));
        }
        if p.f_hz > f.dyn_notch_max_hz as f32 {
            changes.push(rec(
                "dyn_notch_max_hz",
                ParamValue::U16(f.dyn_notch_max_hz),
                ParamValue::U16(clamp_u16((p.f_hz * 1.2) as i32, 200, 1000)),
                format!("Resonance at {:.0} Hz is above dyn_notch_max_hz ({}).", p.f_hz, f.dyn_notch_max_hz),
                ev,
                Confidence::High,
            ));
        }
    }

    // --- Motor noise (>250 Hz) surviving in the filtered gyro → RPM filter / gyro LPF ---
    let motor_surviving: Vec<&&NoisePeak> = filt_peaks
        .iter()
        .filter(|p| p.band == NoiseBand::Motor && p.prominence_db >= 10.0)
        .collect();
    if let Some(p) = motor_surviving.iter().max_by(|a, b| a.prominence_db.partial_cmp(&b.prominence_db).unwrap()) {
        let has_rpm = t.get_raw("dshot_bidir").map(|v| v == "1").unwrap_or(false) && f.rpm_filter_harmonics > 0;
        let ev = vec![peak_ev(p)];
        if !has_rpm {
            changes.push(rec(
                "rpm_filter_harmonics",
                ParamValue::U8(f.rpm_filter_harmonics),
                ParamValue::U8(3),
                format!("Motor noise at {:.0} Hz survives filtering and no RPM filter is active. Enable bidirectional DShot (dshot_bidir = ON) and the RPM filter first — it removes motor noise with the least delay.", p.f_hz),
                ev,
                Confidence::Medium,
            ));
        } else if f.gyro_lpf1_dyn_max_hz > 400 || f.gyro_lpf1_static_hz > 300 {
            let new_max = clamp_u16((f.gyro_lpf1_dyn_max_hz as f32 * 0.8) as i32, 200, 1000);
            changes.push(rec(
                "gyro_lpf1_dyn_max_hz",
                ParamValue::U16(f.gyro_lpf1_dyn_max_hz),
                ParamValue::U16(new_max),
                format!("Motor noise at {:.0} Hz still passes the gyro filter with the RPM filter active; lower the gyro LPF1 dynamic max cutoff.", p.f_hz),
                ev,
                Confidence::Low,
            ));
        }
    }

    // --- D-term noise floor → D-term LPF ---
    let dterm_noisy = dterm_peaks.iter().any(|p| p.f_hz > 120.0 && p.prominence_db >= 12.0);
    if dterm_noisy && f.dterm_lpf1_dyn_max_hz > 120 {
        let p = dterm_peaks.iter().filter(|p| p.f_hz > 120.0).max_by(|a, b| a.prominence_db.partial_cmp(&b.prominence_db).unwrap()).unwrap();
        changes.push(rec(
            "dterm_lpf1_dyn_max_hz",
            ParamValue::U16(f.dterm_lpf1_dyn_max_hz),
            ParamValue::U16(clamp_u16((f.dterm_lpf1_dyn_max_hz as f32 * 0.8) as i32, 60, 500)),
            format!("D-term shows a strong peak at {:.0} Hz ({:+.0} dB); D amplifies noise, so lower the D-term LPF1 dynamic max cutoff (motors run cooler).", p.f_hz, p.prominence_db),
            vec![peak_ev(p)],
            Confidence::Medium,
        ));
    }

    // --- Clean build → relax filtering for less delay ---
    let clean = raw_peaks.iter().all(|p| p.prominence_db < 10.0 || p.band == NoiseBand::Control)
        && filt_peaks.iter().all(|p| p.prominence_db < 8.0)
        && b.quality.has_gyro_raw;
    if clean && f.dyn_notch_count > 1 {
        changes.push(rec(
            "dyn_notch_count",
            ParamValue::U8(f.dyn_notch_count),
            ParamValue::U8(f.dyn_notch_count - 1),
            "Raw gyro spectrum is clean (no peak above +10 dB); each dynamic notch adds delay, so one can be removed.",
            vec![EvidenceRef::Text { note: "no prominent raw-gyro peak".into() }],
            Confidence::Low,
        ));
    }

    if !changes.is_empty() {
        simplified_off(t, &mut out, false, true);
        out.extend(changes);
    }
    out
}

/// PID recommendations from the step responses (Flight B: stick steps).
pub fn pids(t: &BfTune, b: &AnalysisBundle) -> Vec<Recommendation> {
    let mut out = Vec::new();
    let mut changes: Vec<Recommendation> = Vec::new();
    // CHIRP frequency responses first; axes they cover skip the step-response rules.
    let (chirp_recs, chirp_covered) = crate::bf_chirp::rules(t, b);
    let chirp_raw = chirp_covered.iter().any(|c| *c) && t.simplified.pids_mode == 0;
    let names: Vec<(String, String, String, String, String)> = (0..3)
        .map(|k| {
            let ax = ["roll", "pitch", "yaw"][k];
            (format!("p_{ax}"), format!("i_{ax}"), t.cli_d_name(k), t.cli_d_max_name(k), format!("f_{ax}"))
        })
        .collect();

    for s in &b.steps {
        if s.n_segments < 5 || chirp_covered[s.axis.index()] {
            continue;
        }
        let k = s.axis.index();
        let pid = t.pids[k];
        let (pn, in_, dn, dmn, fn_) = (names[k].0.as_str(), names[k].1.as_str(), names[k].2.as_str(), names[k].3.as_str(), names[k].4.as_str());
        let ev = vec![EvidenceRef::Step { axis: s.axis, overshoot: s.overshoot, latency_ms: s.latency_ms }];
        let is_yaw = s.axis == Axis::Yaw;

        // Overshoot: too much P relative to D (roll/pitch) — raise D first, lower P if large.
        if s.overshoot > 1.20 && !is_yaw {
            changes.push(rec(
                dn,
                ParamValue::U8(pid.d),
                ParamValue::U8(clamp_u8(pid.d as i32 + 4, 0, 250)),
                format!("{} overshoots to {:.2} (target ≤ 1.10); more D damps the overshoot.", s.axis.name(), s.overshoot),
                ev.clone(),
                Confidence::Medium,
            ));
            changes.push(rec(
                dmn,
                ParamValue::U8(pid.d_max),
                ParamValue::U8(clamp_u8(pid.d_max as i32 + 4, 0, 250)),
                format!("Raise {} together with D to keep the D_max ratio.", dmn),
                ev.clone(),
                Confidence::Medium,
            ));
            if s.overshoot > 1.35 {
                changes.push(rec(
                    pn,
                    ParamValue::U8(pid.p),
                    ParamValue::U8(clamp_u8(pid.p as i32 - 4, 1, 250)),
                    format!("{} overshoot {:.2} is large; also reduce P slightly.", s.axis.name(), s.overshoot),
                    ev.clone(),
                    Confidence::Medium,
                ));
            }
        } else if s.overshoot > 1.15 && is_yaw {
            changes.push(rec(
                pn,
                ParamValue::U8(pid.p),
                ParamValue::U8(clamp_u8(pid.p as i32 - 5, 1, 250)),
                format!("Yaw overshoots to {:.2}; yaw uses no D, so reduce P.", s.overshoot),
                ev.clone(),
                Confidence::Medium,
            ));
        }

        // Sluggish rise with no overshoot → more P (and FF for stick feel).
        if s.overshoot < 1.03 && s.latency_ms.is_finite() && s.latency_ms > 25.0 {
            changes.push(rec(
                pn,
                ParamValue::U8(pid.p),
                ParamValue::U8(clamp_u8(pid.p as i32 + 4, 1, 250)),
                format!("{} reaches 50 % only after {:.0} ms with no overshoot; raise P for a quicker response.", s.axis.name(), s.latency_ms),
                ev.clone(),
                Confidence::Medium,
            ));
            if pid.ff < 150 {
                changes.push(rec(
                    fn_,
                    ParamValue::U16(pid.ff),
                    ParamValue::U16(clamp_u16(pid.ff as i32 + 15, 0, 1000),),
                    format!("Slow initial rise on {}; a little more feedforward sharpens stick response.", s.axis.name()),
                    ev.clone(),
                    Confidence::Low,
                ));
            }
        }

        // Steady-state below 1 → I too low.
        if s.steady_state < 0.93 {
            changes.push(rec(
                in_,
                ParamValue::U8(pid.i),
                ParamValue::U8(clamp_u8(pid.i as i32 + 8, 1, 250)),
                format!("{} settles at {:.2} instead of 1.0; raise I to remove the steady-state error.", s.axis.name(), s.steady_state),
                ev.clone(),
                Confidence::Medium,
            ));
        }
    }

    if !changes.is_empty() || chirp_raw {
        simplified_off(t, &mut out, true, false);
    }
    // merge duplicate params (keep first); chirp rules take precedence
    let mut seen = std::collections::HashSet::new();
    for c in chirp_recs.into_iter().chain(changes) {
        if seen.insert(c.param.name().to_string()) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_with_pitch_overshoot() -> AnalysisBundle {
        let step = |axis: Axis, overshoot: f32| StepResponse {
            axis, variant: StepVariant::PtStep, t_ms: vec![], mean: vec![], p10: vec![], p90: vec![],
            n_segments: 50, rejected: 0, overshoot, latency_ms: 14.0, settle_ms: None, steady_state: 1.0,
        };
        AnalysisBundle {
            log: LogId("x".into()), quality: LogQuality::default(),
            steps: vec![step(Axis::Roll, 1.05), step(Axis::Pitch, 1.25), step(Axis::Yaw, 1.0)],
            spectra: vec![], spectrograms: vec![], peaks: vec![], anomalies: vec![], freq_resp: vec![],
        }
    }

    #[test]
    fn legacy_firmware_uses_d_min_names() {
        let mut t = BfTune::default();
        t.simplified.pids_mode = 0;
        t.raw.insert("Firmware revision".into(), "Betaflight 4.5.5 (norevision) STM32F7X2".into());
        let names: Vec<String> = pids(&t, &bundle_with_pitch_overshoot()).iter().map(|r| r.param.name().to_string()).collect();
        assert!(names.contains(&"d_min_pitch".to_string()), "{names:?}");
        assert!(names.contains(&"d_pitch".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("d_max_")));
    }

    #[test]
    fn new_firmware_uses_d_max_names() {
        let mut t = BfTune::default();
        t.simplified.pids_mode = 0;
        t.raw.insert("Firmware revision".into(), "Betaflight 2025.12.2 (79065c96b) STM32F7X2".into());
        let names: Vec<String> = pids(&t, &bundle_with_pitch_overshoot()).iter().map(|r| r.param.name().to_string()).collect();
        assert!(names.contains(&"d_max_pitch".to_string()), "{names:?}");
        assert!(names.contains(&"d_pitch".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("d_min_")));
    }
}
