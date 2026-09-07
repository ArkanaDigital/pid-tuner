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
        + usize::from(b.quality.airborne_range_s.is_none()) + usize::from(b.quality.pid_rate_hz.is_none()) + usize::from(b.quality.max_pid_out.is_none());
    assert!(nulls <= expected_nulls, "unexpected nulls in bundle JSON: {nulls} > {expected_nulls}");
}
