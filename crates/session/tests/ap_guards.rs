//! ArduPilot guards evaluated on the real fixtures. Expected values are the
//! ones pymavlink reports for those files (see fixtures/ap/*.golden.json):
//! the 4.6.3 log has LOG_BITMASK = 180222 (bit 0 clear) and PIDx at 10 Hz.
use domain::*;
use session::*;
use std::path::PathBuf;

fn fixture(name: &str) -> Option<Vec<u8>> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ap").join(name)).ok()
}

fn engine() -> (tempfile::TempDir, SessionEngine) {
    let d = tempfile::tempdir().unwrap();
    let e = SessionEngine::create(SessionStore::new(d.path()), "ap".into(), Mode::Offline).unwrap();
    (d, e)
}

fn attach(e: &mut SessionEngine, which: Flight, bytes: &[u8]) -> FlightLog {
    let log = ap_ingest::ingest(bytes, 0, &Default::default()).unwrap();
    let bundle = analysis::analyze(&log, &Default::default(), |_| {});
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), bytes).unwrap();
    e.attach_log(which, tmp.path(), 0, &log, &bundle).unwrap();
    log
}

fn guard<'a>(g: &'a [GuardResult], id: &str) -> &'a GuardResult {
    g.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("guard {id} missing in {:?}", g.iter().map(|x| &x.id).collect::<Vec<_>>()))
}

#[test]
fn ten_hz_pid_log_fails_loop_rate_guard_with_bit0_hint() {
    let Some(bytes) = fixture("copter_4.6.3_quad_pid_msgs.bin") else { return };
    let (_d, mut e) = engine();
    attach(&mut e, Flight::B, &bytes);
    let g = e.guards(Step::ImportB, None);
    // firmware is ArduCopter: Betaflight-only guards must not appear
    assert!(g.iter().all(|x| x.id != "log_rate" && x.id != "steps"), "{:?}", g.iter().map(|x| &x.id).collect::<Vec<_>>());
    let pr = guard(&g, "ap_pid_rate");
    match &pr.outcome {
        GuardOutcome::Fail { message, fix_hint } => {
            assert!(message.contains("10 Hz") && message.contains("400 Hz"), "{message}");
            assert!(fix_hint.as_deref().unwrap().contains("bit 0"), "{fix_hint:?}");
        }
        o => panic!("{o:?}"),
    }
    assert!(!guard(&g, "ap_steps").satisfied());
    assert!(guard(&g, "ap_saturation").satisfied(), "{:?}", guard(&g, "ap_saturation").outcome);
}

#[test]
fn batch_sampler_log_passes_isbh_and_hover_guards() {
    let Some(bytes) = fixture("copter_4.5.5_tarot_x4_batch_imu.bin") else { return };
    let (_d, mut e) = engine();
    let log = attach(&mut e, Flight::A, &bytes);
    assert!(log.gyro_hr.iter().map(|t| t.batches.len()).sum::<usize>() >= 20);
    let g = e.guards(Step::ImportA, None);
    assert!(guard(&g, "ap_isbh").satisfied(), "{:?}", guard(&g, "ap_isbh").outcome);
    assert!(guard(&g, "ap_hover").satisfied(), "{:?}", guard(&g, "ap_hover").outcome);
    assert!(guard(&g, "duration").satisfied());
    assert!(g.iter().all(|x| x.id != "raw_gyro" && x.id != "hover"));
}

#[test]
fn no_batch_log_fails_isbh_guard() {
    let Some(bytes) = fixture("copter_4.4.2_auto_jitter.bin") else { return };
    let (_d, mut e) = engine();
    attach(&mut e, Flight::A, &bytes);
    let g = e.guards(Step::ImportA, None);
    match &guard(&g, "ap_isbh").outcome {
        GuardOutcome::Fail { fix_hint, .. } => assert!(fix_hint.as_deref().unwrap().contains("INS_LOG_BAT_MASK")),
        o => panic!("{o:?}"),
    }
}

#[test]
fn autotune_result_guard_detects_unsaved_axes() {
    let Some(bytes) = fixture("copter_4.6.3_quad_pid_msgs.bin") else { return };
    let (_d, mut e) = engine();
    e.set_pid_strategy(PidStrategy::Autotune).unwrap();
    // B and C identical → nothing changed on any axis
    attach(&mut e, Flight::B, &bytes);
    attach(&mut e, Flight::C, &bytes);
    let g = e.guards(Step::ImportC, None);
    match &guard(&g, "autotune_result").outcome {
        GuardOutcome::Fail { message, .. } => {
            assert!(message.contains("RLL") && message.contains("PIT") && message.contains("unchanged"), "{message}");
        }
        o => panic!("{o:?}"),
    }
    // Heuristic strategy: guard is a no-op
    e.set_pid_strategy(PidStrategy::Heuristic).unwrap();
    assert!(guard(&e.guards(Step::ImportC, None), "autotune_result").satisfied());
}

#[test]
fn fc_supported_and_preflight_bits_follow_ardupilot_defines() {
    let (_d, e) = engine();
    let mut fc = FcStatus { connected: true, ..Default::default() };
    fc.firmware = Some(Firmware::ArduCopter { version: "4.3.8".into() });
    let supported = |fc: &FcStatus| guard(&e.guards(Step::Connect, Some(fc)), "fc_supported").satisfied();
    assert!(!supported(&fc));
    fc.firmware = Some(Firmware::ArduCopter { version: "4.5.5".into() });
    assert!(supported(&fc));

    // LOG_BITMASK of the 4.6.3 fixture: bit 0 (ATTITUDE_FAST) clear → fail
    fc.log_bitmask = Some(180222);
    fc.batch_configured = Some(false);
    let g = e.guards(Step::Preflight, Some(&fc));
    assert!(!guard(&g, "ap_log_bitmask").satisfied());
    assert!(!guard(&g, "ap_batch").satisfied());
    assert!(g.iter().all(|x| x.id != "log_rate" && x.id != "raw_gyro_logging"), "{:?}", g.iter().map(|x| &x.id).collect::<Vec<_>>());

    fc.log_bitmask = Some(180222 | domain::ap_consts::LOG_BIT_ATTITUDE_FAST | domain::ap_consts::LOG_BIT_PID | domain::ap_consts::LOG_BIT_IMU_RAW);
    fc.batch_configured = Some(true);
    let g = e.guards(Step::Preflight, Some(&fc));
    assert!(guard(&g, "ap_log_bitmask").satisfied());
    assert!(guard(&g, "ap_batch").satisfied());
}
