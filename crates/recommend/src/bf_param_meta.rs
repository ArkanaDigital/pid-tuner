//! Bounds of the Betaflight CLI settings this app may write (ours or the AI's
//! proposals). Ranges follow `src/main/cli/settings.c` (min/max of each
//! `VAR_*` entry); anything not listed is rejected, never clamped.

use domain::ParamValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BfKind {
    U8,
    U16,
    /// integer-valued enum (`ParamValue::Enum`)
    Enum,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BfParamMeta {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub kind: BfKind,
    pub requires_reboot: bool,
    /// Tuning phase the parameter belongs to.
    pub phase: BfPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BfPhase {
    Pids,
    Filters,
}

const AXES: [&str; 3] = ["roll", "pitch", "yaw"];

/// (template with `{ax}`, min, max, kind, phase). Per-axis templates expand to three names.
const TABLE: &[(&str, f64, f64, BfKind, BfPhase)] = &[
    ("p_{ax}", 0.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("i_{ax}", 0.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("d_{ax}", 0.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("d_min_{ax}", 0.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("d_max_{ax}", 0.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("f_{ax}", 0.0, 2000.0, BfKind::U16, BfPhase::Pids),
    ("d_max_gain", 0.0, 100.0, BfKind::U8, BfPhase::Pids),
    ("d_max_advance", 0.0, 200.0, BfKind::U8, BfPhase::Pids),
    ("d_min_gain", 0.0, 100.0, BfKind::U8, BfPhase::Pids),
    ("d_min_advance", 0.0, 200.0, BfKind::U8, BfPhase::Pids),
    ("anti_gravity_gain", 0.0, 250.0, BfKind::U16, BfPhase::Pids),
    ("iterm_relax", 0.0, 2.0, BfKind::Enum, BfPhase::Pids),
    ("iterm_relax_type", 0.0, 1.0, BfKind::Enum, BfPhase::Pids),
    ("iterm_relax_cutoff", 1.0, 100.0, BfKind::U8, BfPhase::Pids),
    (
        "feedforward_smooth_factor",
        0.0,
        95.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "feedforward_jitter_factor",
        0.0,
        20.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    ("feedforward_boost", 0.0, 50.0, BfKind::U8, BfPhase::Pids),
    (
        "feedforward_transition",
        0.0,
        100.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    ("tpa_rate", 0.0, 100.0, BfKind::U8, BfPhase::Pids),
    ("tpa_breakpoint", 1000.0, 2000.0, BfKind::U16, BfPhase::Pids),
    ("thrust_linear", 0.0, 150.0, BfKind::U8, BfPhase::Pids),
    (
        "simplified_pids_mode",
        0.0,
        2.0,
        BfKind::Enum,
        BfPhase::Pids,
    ),
    (
        "simplified_master_multiplier",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    ("simplified_pi_gain", 25.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("simplified_i_gain", 25.0, 250.0, BfKind::U8, BfPhase::Pids),
    ("simplified_d_gain", 25.0, 250.0, BfKind::U8, BfPhase::Pids),
    (
        "simplified_dmax_gain",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "simplified_feedforward_gain",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "simplified_pitch_pi_gain",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "simplified_pitch_d_gain",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "simplified_roll_pitch_ratio",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Pids,
    ),
    (
        "simplified_gyro_filter",
        0.0,
        1.0,
        BfKind::Bool,
        BfPhase::Filters,
    ),
    (
        "simplified_gyro_filter_multiplier",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Filters,
    ),
    (
        "simplified_dterm_filter",
        0.0,
        1.0,
        BfKind::Bool,
        BfPhase::Filters,
    ),
    (
        "simplified_dterm_filter_multiplier",
        25.0,
        250.0,
        BfKind::U8,
        BfPhase::Filters,
    ),
    (
        "gyro_lpf1_static_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "gyro_lpf2_static_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "gyro_lpf1_dyn_min_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "gyro_lpf1_dyn_max_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    ("gyro_lpf1_type", 0.0, 3.0, BfKind::Enum, BfPhase::Filters),
    ("gyro_lpf2_type", 0.0, 3.0, BfKind::Enum, BfPhase::Filters),
    ("gyro_notch1_hz", 0.0, 1000.0, BfKind::U16, BfPhase::Filters),
    (
        "gyro_notch1_cutoff",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    ("gyro_notch2_hz", 0.0, 1000.0, BfKind::U16, BfPhase::Filters),
    (
        "gyro_notch2_cutoff",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "dterm_lpf1_static_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "dterm_lpf2_static_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "dterm_lpf1_dyn_min_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "dterm_lpf1_dyn_max_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    ("dterm_lpf1_type", 0.0, 3.0, BfKind::Enum, BfPhase::Filters),
    ("dterm_lpf2_type", 0.0, 3.0, BfKind::Enum, BfPhase::Filters),
    ("dterm_notch_hz", 0.0, 1000.0, BfKind::U16, BfPhase::Filters),
    (
        "dterm_notch_cutoff",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    ("dyn_notch_count", 0.0, 5.0, BfKind::U8, BfPhase::Filters),
    ("dyn_notch_q", 1.0, 1000.0, BfKind::U16, BfPhase::Filters),
    (
        "dyn_notch_min_hz",
        20.0,
        250.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "dyn_notch_max_hz",
        200.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "rpm_filter_harmonics",
        0.0,
        3.0,
        BfKind::U8,
        BfPhase::Filters,
    ),
    (
        "rpm_filter_min_hz",
        50.0,
        200.0,
        BfKind::U8,
        BfPhase::Filters,
    ),
    ("rpm_filter_q", 250.0, 3000.0, BfKind::U16, BfPhase::Filters),
    (
        "rpm_filter_fade_range_hz",
        0.0,
        1000.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    (
        "rpm_filter_lpf_hz",
        100.0,
        500.0,
        BfKind::U16,
        BfPhase::Filters,
    ),
    ("yaw_lowpass_hz", 0.0, 500.0, BfKind::U16, BfPhase::Filters),
];

fn split_axis(name: &str) -> Option<(String, &'static str)> {
    for ax in AXES {
        if let Some(stem) = name.strip_suffix(&format!("_{ax}")) {
            return Some((format!("{stem}_{{ax}}"), ax));
        }
    }
    None
}

pub fn bf_param_meta(name: &str) -> Option<BfParamMeta> {
    let key = match split_axis(name) {
        Some((tpl, _)) if TABLE.iter().any(|t| t.0 == tpl) => tpl,
        _ => name.to_string(),
    };
    TABLE.iter().find(|t| t.0 == key).map(|t| BfParamMeta {
        name: t.0,
        min: t.1,
        max: t.2,
        kind: t.3,
        requires_reboot: false,
        phase: t.4,
    })
}

pub fn is_bf_param(name: &str) -> bool {
    bf_param_meta(name).is_some()
}

/// Typed, clamped value for `name`; `None` when the parameter is not allowed.
pub fn clamp_bf(name: &str, v: f64) -> Option<ParamValue> {
    let m = bf_param_meta(name)?;
    let x = if v.is_finite() {
        v.clamp(m.min, m.max).round()
    } else {
        m.min
    };
    Some(match m.kind {
        BfKind::U8 => ParamValue::U8(x as u8),
        BfKind::U16 => ParamValue::U16(x as u16),
        BfKind::Enum => ParamValue::Enum(x as u8),
        BfKind::Bool => ParamValue::Bool(x >= 0.5),
    })
}

/// Every name in the table, expanded (for tests and the UI).
pub fn all_names() -> Vec<String> {
    let mut out = Vec::new();
    for t in TABLE {
        if t.0.contains("{ax}") {
            for ax in AXES {
                out.push(t.0.replace("{ax}", ax));
            }
        } else {
            out.push(t.0.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::*;

    #[test]
    fn axis_templates_and_bounds() {
        let m = bf_param_meta("d_max_pitch").unwrap();
        assert_eq!(
            (m.min, m.max, m.kind, m.phase),
            (0.0, 250.0, BfKind::U8, BfPhase::Pids)
        );
        assert_eq!(bf_param_meta("f_yaw").unwrap().max, 2000.0);
        assert!(bf_param_meta("p_throttle").is_none());
        assert!(bf_param_meta("gyro_lpf1_static_hz").is_some());
        assert!(bf_param_meta("blackbox_disable_setpoint").is_none());
        assert_eq!(clamp_bf("d_roll", 999.0), Some(ParamValue::U8(250)));
        assert_eq!(clamp_bf("dyn_notch_min_hz", 5.0), Some(ParamValue::U16(20)));
        assert_eq!(
            clamp_bf("simplified_pids_mode", 7.0),
            Some(ParamValue::Enum(2))
        );
        assert_eq!(
            clamp_bf("simplified_gyro_filter", 1.0),
            Some(ParamValue::Bool(true))
        );
        assert_eq!(clamp_bf("f_roll", f64::NAN), Some(ParamValue::U16(0)));
        assert_eq!(clamp_bf("nope", 1.0), None);
        assert!(all_names().contains(&"i_pitch".to_string()));
    }

    #[test]
    fn every_name_the_rules_emit_is_in_the_table() {
        let mut t = BfTune::default();
        t.simplified.pids_mode = 2;
        t.simplified.gyro_filter = true;
        t.simplified.dterm_filter = true;
        t.raw.insert(
            "Firmware revision".into(),
            "Betaflight 4.5.1 (x) STM32F405".into(),
        );
        let step = |axis: Axis, overshoot: f32, lat: f32, ss: f32| StepResponse {
            axis,
            variant: StepVariant::PtStep,
            t_ms: vec![],
            mean: vec![],
            p10: vec![],
            p90: vec![],
            n_segments: 50,
            rejected: 0,
            overshoot,
            latency_ms: lat,
            settle_ms: None,
            steady_state: ss,
        };
        let peaks = vec![
            NoisePeak {
                axis: Axis::Roll,
                kind: SpectrumKind::GyroFilt,
                f_hz: 180.0,
                psd_db: 5.0,
                prominence_db: 20.0,
                band: NoiseBand::Frame,
            },
            NoisePeak {
                axis: Axis::Roll,
                kind: SpectrumKind::GyroFilt,
                f_hz: 420.0,
                psd_db: 5.0,
                prominence_db: 20.0,
                band: NoiseBand::Motor,
            },
            NoisePeak {
                axis: Axis::Pitch,
                kind: SpectrumKind::DTerm,
                f_hz: 300.0,
                psd_db: 5.0,
                prominence_db: 20.0,
                band: NoiseBand::Motor,
            },
        ];
        let b = AnalysisBundle {
            log: LogId("x".into()),
            quality: LogQuality {
                has_gyro_raw: true,
                ..Default::default()
            },
            steps: vec![
                step(Axis::Roll, 1.3, 14.0, 0.9),
                step(Axis::Pitch, 1.0, 40.0, 1.0),
                step(Axis::Yaw, 1.2, 14.0, 1.0),
            ],
            spectra: vec![],
            spectrograms: vec![],
            peaks,
            anomalies: vec![],
            freq_resp: vec![],
        };
        let mut names: Vec<String> = crate::bf::pids(&t, &b)
            .iter()
            .map(|r| r.param.name().to_string())
            .collect();
        names.extend(
            crate::bf::filters(&t, &b)
                .iter()
                .map(|r| r.param.name().to_string()),
        );
        let mut t2 = t.clone();
        t2.raw.insert(
            "Firmware revision".into(),
            "Betaflight 2026.6.1 (x) STM32F405".into(),
        );
        names.extend(
            crate::bf::pids(&t2, &b)
                .iter()
                .map(|r| r.param.name().to_string()),
        );
        assert!(!names.is_empty());
        for n in names {
            assert!(is_bf_param(&n), "{n} missing from bf_param_meta");
        }
    }
}
