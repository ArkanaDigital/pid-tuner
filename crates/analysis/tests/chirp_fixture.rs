//! Golden-style checks on the real CHIRP fixture (pichim/bf_controller_tuning,
//! Betaflight 2026.6.0-alpha, whole flight in ANGLE mode). Numbers were taken
//! from the first verified run; tolerances follow the plan (bandwidth ±2 %,
//! phase margin ±1°, resonant peak ±0.2 dB).
use domain::*;
use std::path::PathBuf;

fn fixture() -> Option<Vec<u8>> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bf/bf_2025.12_chirp_gyro_angle.TXT")).ok()
}

#[test]
fn chirp_fixture_segments_and_yaw_metrics() {
    let Some(bytes) = fixture() else { return };
    let log = log_ingest::ingest(&bytes, 0).unwrap();
    assert_eq!(log.meta.debug_mode.as_deref(), Some("CHIRP"));
    assert_eq!(log.meta.headers.get("bf.debug_mode_raw").map(String::as_str), Some("97"));
    assert!((log.fs_hz - 1996.0).abs() < 5.0, "{}", log.fs_hz);
    let chirp = log.chirp.as_ref().expect("chirp info");
    let cfg = chirp.config.as_ref().unwrap();
    assert!((cfg.f_start_hz - 0.2).abs() < 1e-6 && (cfg.f_end_hz - 600.0).abs() < 1e-6 && cfg.time_s == 20.0 && cfg.amplitude == [230, 230, 180]);
    assert!(chirp.debug_is_chirp);
    assert_eq!(chirp.segments.len(), 3, "{:?}", chirp.segments);
    let axes: Vec<Axis> = chirp.segments.iter().map(|s| s.axis).collect();
    assert_eq!(axes, vec![Axis::Roll, Axis::Pitch, Axis::Yaw]);
    for s in &chirp.segments {
        assert_eq!(s.source, ChirpGate::Both);
        assert!((s.f_start_hz - 0.2).abs() < 0.05 && s.f_end_hz > 590.0, "{s:?}");
        assert_eq!(s.angle_mode, s.axis != Axis::Yaw, "{s:?}");
        assert!(s.t1_s - s.t0_s > 70.0);
    }
    let b = analysis::analyze(&log, &Default::default(), |_| {});
    assert_eq!(b.freq_resp.len(), 3);
    let yaw = b.freq_resp.iter().find(|f| f.axis == Axis::Yaw).unwrap();
    assert!(!yaw.angle_mode);
    assert_eq!(yaw.n_sweeps, 4);
    assert!(yaw.n_windows >= 250);
    let m = &yaw.metrics;
    assert!(m.coherence_mean > 0.9, "{}", m.coherence_mean);
    assert!(((m.bandwidth_hz - 20.2) / 20.2).abs() < 0.02, "bandwidth {}", m.bandwidth_hz);
    assert!(((m.crossover_hz - 12.6) / 12.6).abs() < 0.02, "crossover {}", m.crossover_hz);
    assert!((m.phase_margin_deg - 66.1).abs() < 1.0, "PM {}", m.phase_margin_deg);
    assert!((m.resonant_peak_db - 0.7).abs() < 0.2, "Mr {}", m.resonant_peak_db);
    assert!((m.loop_delay_ms - 4.67).abs() < 0.3, "delay {}", m.loop_delay_ms);
    // roll/pitch: ANGLE-mode sweeps are analysed but flagged, and their coherence is poor
    for ax in [Axis::Roll, Axis::Pitch] {
        let fr = b.freq_resp.iter().find(|f| f.axis == ax).unwrap();
        assert!(fr.angle_mode);
        assert!(fr.metrics.coherence_mean < 0.6, "{:?} {}", ax, fr.metrics.coherence_mean);
    }
    assert_eq!(b.quality.chirp_sweeps_per_axis, [4, 4, 4]);
    assert!(b.quality.chirp_coherence_per_axis[2] > 0.9 && b.quality.chirp_coherence_per_axis[0] < 0.6);
    // the excitation lives in the setpoint: no phantom oscillation anomalies
    assert!(b.anomalies.iter().all(|a| a.kind != AnomalyKind::Oscillation), "{:?}", b.anomalies);
}
