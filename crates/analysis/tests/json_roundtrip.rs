//! Bundles are cached as JSON (session dir) and sent to the UI: every value,
//! including NaN latency for axes without step segments, must survive a
//! serialize → deserialize round trip.
use std::path::PathBuf;

fn fixture(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(rel)).ok()
}

#[test]
fn step_response_nan_latency_roundtrips_as_null() {
    let s = domain::StepResponse { axis: domain::Axis::Roll, variant: domain::StepVariant::PtStep, t_ms: vec![], mean: vec![], p10: vec![], p90: vec![], n_segments: 0, rejected: 3, overshoot: 0.0, latency_ms: f32::NAN, settle_ms: None, steady_state: 0.0 };
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"latency_ms\":null"), "{json}");
    let back: domain::StepResponse = serde_json::from_str(&json).unwrap();
    assert!(back.latency_ms.is_nan());
    let s2 = domain::StepResponse { latency_ms: 19.5, ..s };
    let back: domain::StepResponse = serde_json::from_str(&serde_json::to_string(&s2).unwrap()).unwrap();
    assert_eq!(back.latency_ms, 19.5);
}

#[test]
fn bundle_json_roundtrip_on_hover_log() {
    // steady hover → few/no step segments on some axis
    let Some(bytes) = fixture("bf/bf_2025.12.2_speedybeef7v3_steadyhover.BFL").or_else(|| fixture("ap/copter_4.6.3_quad_pid_msgs.bin")) else { return };
    let log = log_ingest::ingest(&bytes, 0).unwrap();
    let b = analysis::analyze(&log, &Default::default(), |_| {});
    let json = serde_json::to_vec(&b).expect("serialize");
    let back: domain::AnalysisBundle = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(back.steps.len(), b.steps.len());
    for (a, c) in b.steps.iter().zip(&back.steps) {
        assert!(a.latency_ms.is_nan() == c.latency_ms.is_nan());
        assert_eq!(a.n_segments, c.n_segments);
    }
    // no other NaN/inf can hide in the JSON as null
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    fn count_null(v: &serde_json::Value) -> usize {
        match v {
            serde_json::Value::Null => 1,
            serde_json::Value::Array(a) => a.iter().map(count_null).sum(),
            serde_json::Value::Object(o) => o.values().map(count_null).sum(),
            _ => 0,
        }
    }
    let nulls = count_null(&v);
    let expected_nulls = b.steps.iter().filter(|s| s.latency_ms.is_nan()).count() + b.steps.iter().filter(|s| s.settle_ms.is_none()).count()
        + usize::from(b.quality.airborne_range_s.is_none()) + usize::from(b.quality.pid_rate_hz.is_none()) + usize::from(b.quality.max_pid_out.is_none())
        + b.anomalies.iter().map(|a| usize::from(a.axis.is_none()) + usize::from(a.motor.is_none())).sum::<usize>()
        + b.freq_resp.iter().map(|fr| {
            let vecs = [&fr.h_mag_db, &fr.h_phase_deg, &fr.coherence, &fr.l_mag_db, &fr.l_phase_deg, &fr.s_mag_db];
            let v: usize = vecs.iter().map(|v| v.iter().filter(|x| !x.is_finite()).count()).sum();
            let m = &fr.metrics;
            let scalars = [m.bandwidth_hz, m.crossover_hz, m.phase_margin_deg, m.max_phase_margin_deg, m.resonant_peak_db, m.resonant_peak_hz, m.loop_delay_ms, m.low_freq_err_db, m.coherence_mean, m.noise_floor_hz, m.sens_peak_db, m.sens_peak_hz, m.step_overshoot, m.step_rise_ms, m.step_settle_ms];
            let t: usize = m.targets.iter().map(|t| [t.crossover_hz, t.gain_to_target, t.gain_for_sens_limit].iter().filter(|x| !x.is_finite()).count()).sum();
            v + scalars.iter().filter(|x| !x.is_finite()).count() + t
        }).sum::<usize>();
    assert!(nulls <= expected_nulls, "unexpected nulls in bundle JSON: {nulls} > {expected_nulls}");
}

#[test]
fn frequency_response_nan_vectors_roundtrip() {
    let fr = domain::FrequencyResponse {
        axis: domain::Axis::Pitch,
        angle_mode: false,
        f_hz: vec![0.0, 1.0, 2.0],
        h_mag_db: vec![f32::NAN, -1.0, -3.0],
        h_phase_deg: vec![0.0, f32::NEG_INFINITY, -90.0],
        coherence: vec![0.0, 0.5, 1.0],
        l_mag_db: vec![f32::NAN, f32::NAN, 2.0],
        l_phase_deg: vec![f32::NAN, f32::NAN, -120.0],
        s_mag_db: vec![0.0, 0.0, 3.0],
        step_t_ms: vec![0.0, 0.5],
        step: vec![0.0, 0.3],
        fs_hz: 2000.0,
        segment_size: 1024,
        n_windows: 12,
        n_sweeps: 2,
        sweep_seconds: 20.0,
        metrics: domain::FrMetrics {
            bandwidth_hz: 25.0, crossover_hz: f32::NAN, phase_margin_deg: f32::NAN, max_phase_margin_deg: 80.0, resonant_peak_db: 1.0, resonant_peak_hz: 30.0,
            loop_delay_ms: f32::NAN, low_freq_err_db: 0.1, coherence_mean: 0.9, noise_floor_hz: f32::NAN, sens_peak_db: 2.0, sens_peak_hz: 40.0,
            step_overshoot: 1.05, step_rise_ms: 12.0, step_settle_ms: 40.0,
            targets: vec![domain::FrTarget { pm_deg: 60.0, crossover_hz: f32::NAN, gain_to_target: f32::NAN, gain_for_sens_limit: 1.2 }],
        },
    };
    let json = serde_json::to_string(&fr).unwrap();
    assert!(json.contains("\"h_mag_db\":[null,-1.0,-3.0]"), "{json}");
    let back: domain::FrequencyResponse = serde_json::from_str(&json).unwrap();
    assert!(back.h_mag_db[0].is_nan() && back.h_phase_deg[1].is_nan() && back.l_mag_db[1].is_nan());
    assert!(back.metrics.crossover_hz.is_nan() && back.metrics.targets[0].crossover_hz.is_nan());
    assert_eq!(back.metrics.targets[0].gain_for_sens_limit, 1.2);
}
