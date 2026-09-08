//! Wizard guards on a Betaflight CHIRP log imported as Flight B.
use session::*;
use std::path::PathBuf;

fn fixture() -> Option<Vec<u8>> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/bf/bf_2025.12_chirp_gyro_angle.TXT"),
    )
    .ok()
}

#[test]
fn chirp_log_sets_pid_source_and_guards_report_incoherent_axes() {
    let Some(bytes) = fixture() else { return };
    let d = tempfile::tempdir().unwrap();
    let mut e =
        SessionEngine::create(SessionStore::new(d.path()), "chirp".into(), Mode::Offline).unwrap();
    let log = log_ingest::ingest(&bytes, 0).unwrap();
    let bundle = analysis::analyze(&log, &Default::default(), |_| {});
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &bytes).unwrap();
    e.attach_log(Flight::B, tmp.path(), 0, &log, &bundle)
        .unwrap();
    assert_eq!(e.session.pid_source, PidSource::Chirp);
    let g = e.guards(Step::ImportB, None);
    let cq = g
        .iter()
        .find(|x| x.id == "chirp_quality")
        .expect("chirp_quality guard");
    match &cq.outcome {
        GuardOutcome::Fail { message, fix_hint } => {
            assert!(
                message.contains("roll") && message.contains("pitch") && !message.contains("yaw"),
                "{message}"
            );
            assert!(fix_hint.as_deref().unwrap().contains("ACRO"));
        }
        o => panic!("{o:?}"),
    }
    // yaw is measured by the sweep; roll/pitch still have plenty of stick-step segments in this log
    assert!(g.iter().find(|x| x.id == "steps").unwrap().satisfied());
    assert!(cq.can_override);
    // recommendations: chirp rules only on yaw (roll/pitch sweeps are ANGLE mode)
    let recs = recommend::recommend_for_log(&log, &bundle, recommend::Phase::Pids);
    let names: Vec<&str> = recs.iter().map(|r| r.param.name()).collect();
    assert!(
        names
            .iter()
            .all(|n| !n.contains("filter") && !n.contains("lpf") && !n.contains("notch")),
        "{names:?}"
    );
    assert!(recs.iter().any(|r| r.reason.contains("ANGLE")), "{names:?}");
}

#[test]
fn chirp_quality_guard_absent_without_chirp_data() {
    let d = tempfile::tempdir().unwrap();
    let e =
        SessionEngine::create(SessionStore::new(d.path()), "plain".into(), Mode::Offline).unwrap();
    // without an import the guard reports "Import first."
    let g = e.guards(Step::ImportB, None);
    assert!(g.iter().any(|x| x.id == "chirp_quality"));
}
