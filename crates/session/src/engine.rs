//! Drives one session through the wizard.

use crate::guards::{self, FcStatus, GuardCtx, GuardResult};
use crate::model::*;
use crate::{Result, SessionError, SessionStore};
use chrono::Utc;
use domain::*;
use uuid::Uuid;

pub struct SessionEngine {
    pub store: SessionStore,
    pub session: Session,
}

impl SessionEngine {
    pub fn create(store: SessionStore, name: String, mode: Mode) -> Result<Self> {
        let session = Session::new(name, mode);
        store.save(&session)?;
        Ok(Self { store, session })
    }

    pub fn open(store: SessionStore, id: Uuid) -> Result<Self> {
        let session = store.load(id)?;
        Ok(Self { store, session })
    }

    fn touch(&mut self) -> Result<()> {
        self.session.updated_at = Utc::now();
        self.store.save(&self.session)
    }

    pub fn guards(&self, step: Step, fc: Option<&FcStatus>) -> Vec<GuardResult> {
        guards::evaluate(step, &GuardCtx { session: &self.session, fc })
    }

    pub fn snapshot(&self, fc: Option<&FcStatus>) -> SessionSnapshot {
        let steps = Step::ALL
            .iter()
            .map(|&s| StepView {
                step: s,
                title: s.title().to_string(),
                status: self.session.status[&s],
                needs_fc: s.needs_fc(),
                guards: if s == self.session.current { self.guards(s, fc) } else { Vec::new() },
            })
            .collect::<Vec<_>>();
        let cur = self.session.current;
        let can_next = self.guards(cur, fc).iter().all(|g| g.satisfied()) && cur.next().is_some();
        let can_back = cur.prev().is_some();
        SessionSnapshot { session: self.session.clone(), steps, can_next, can_back }
    }

    /// Advance if every guard of the current step is satisfied.
    pub fn next(&mut self, fc: Option<&FcStatus>) -> Result<Step> {
        let cur = self.session.current;
        if let Some(g) = self.guards(cur, fc).into_iter().find(|g| !g.satisfied()) {
            let msg = match &g.outcome {
                guards::GuardOutcome::Fail { message, .. } | guards::GuardOutcome::NeedsAction { message } => message.clone(),
                guards::GuardOutcome::Pass => String::new(),
            };
            return Err(SessionError::GuardFailed(g.id, msg));
        }
        let Some(mut nxt) = cur.next() else { return Ok(cur) };
        self.session.status.insert(cur, StepStatus::Passed);
        // Offline sessions skip FC-only steps that have nothing to do.
        while self.session.mode == Mode::Offline && matches!(nxt, Step::Connect | Step::Preflight) {
            self.session.status.insert(nxt, StepStatus::Skipped);
            nxt = nxt.next().unwrap();
        }
        self.session.status.insert(nxt, StepStatus::Active);
        self.session.current = nxt;
        self.touch()?;
        Ok(nxt)
    }

    /// Go back one step (previous step becomes Active again; its guards will be re-evaluated).
    pub fn back(&mut self) -> Result<Step> {
        let cur = self.session.current;
        let mut prev = cur.prev().ok_or_else(|| SessionError::Invalid("already at the first step".into()))?;
        while self.session.status[&prev] == StepStatus::Skipped {
            prev = prev.prev().ok_or_else(|| SessionError::Invalid("no previous step".into()))?;
        }
        self.session.status.insert(cur, StepStatus::Locked);
        self.session.status.insert(prev, StepStatus::Active);
        self.session.current = prev;
        self.touch()?;
        Ok(prev)
    }

    /// Jump to an already-passed step to review it (does not change statuses of later steps).
    pub fn goto(&mut self, step: Step) -> Result<Step> {
        match self.session.status[&step] {
            StepStatus::Locked => Err(SessionError::Invalid(format!("{} is locked", step.title()))),
            _ => {
                self.session.current = step;
                self.session.status.insert(step, StepStatus::Active);
                self.touch()?;
                Ok(step)
            }
        }
    }

    pub fn override_guard(&mut self, guard_id: &str, reason: &str) -> Result<()> {
        let step = self.session.current;
        if !guards::can_override(step, guard_id) {
            return Err(SessionError::NotOverridable(guard_id.to_string()));
        }
        if reason.trim().len() < 5 {
            return Err(SessionError::Invalid("an override needs a written reason".into()));
        }
        self.session.overrides.retain(|o| !(o.step == step && o.guard_id == guard_id));
        self.session.overrides.push(Override { step, guard_id: guard_id.to_string(), reason: reason.trim().to_string(), at: Utc::now() });
        self.touch()
    }

    pub fn mark_flight_done(&mut self, which: Flight, done: bool) -> Result<()> {
        if done {
            self.session.flight_done.insert(which, Utc::now());
        } else {
            self.session.flight_done.remove(&which);
        }
        self.touch()
    }

    /// Attach an ingested + analysed log to a flight. The caller has already
    /// run the analysis; the bundle is cached as JSON in the session dir.
    pub fn attach_log(
        &mut self,
        which: Flight,
        original_path: &std::path::Path,
        session_index: usize,
        log: &FlightLog,
        bundle: &AnalysisBundle,
    ) -> Result<()> {
        let label = format!("flight_{}", format!("{which:?}").to_lowercase());
        let log_file = self.store.import_file(self.session.id, original_path, &label)?;
        let bundle_file = format!("analysis/{}.json", log.id.0);
        self.store.write_rel(self.session.id, &bundle_file, &serde_json::to_vec(bundle)?)?;
        self.session.flights.insert(
            which,
            FlightRecord {
                log_file,
                original_path: original_path.display().to_string(),
                session_index,
                log_id: log.id.clone(),
                imported_at: Utc::now(),
                firmware: log.firmware.clone(),
                craft_name: log.meta.craft_name.clone(),
                fs_hz: log.fs_hz,
                duration_s: log.duration_s(),
                quality: bundle.quality.clone(),
                warnings: log.meta.warnings.clone(),
                tune: log.tune_at_log.clone(),
                bundle_file,
            },
        );
        if self.session.firmware.is_none() {
            self.session.firmware = Some(log.firmware.clone());
        }
        self.touch()
    }

    pub fn bundle(&self, which: Flight) -> Result<Option<AnalysisBundle>> {
        let Some(r) = self.session.flights.get(&which) else { return Ok(None) };
        let bytes = self.store.read_rel(self.session.id, &r.bundle_file)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn set_recs(&mut self, phase: ApplyPhase, recs: Vec<Recommendation>) -> Result<()> {
        *self.session.recs_mut(phase) = recs;
        self.touch()
    }

    pub fn set_rec(&mut self, phase: ApplyPhase, id: Uuid, accepted: bool, new_value: Option<ParamValue>) -> Result<()> {
        if let Some(r) = self.session.recs_mut(phase).iter_mut().find(|r| r.id == id) {
            r.accepted = accepted;
            if let Some(v) = new_value {
                r.new = v;
            }
        }
        self.touch()
    }

    pub fn record_apply(&mut self, phase: ApplyPhase, verified: bool, method: &str, notes: Option<String>) -> Result<()> {
        let applied: Vec<Recommendation> = self.session.recs(phase).iter().filter(|r| r.accepted).cloned().collect();
        self.session.applies.push(ApplyRecord { phase, at: Utc::now(), applied, verified, method: method.to_string(), notes });
        self.touch()
    }

    pub fn add_snapshot(&mut self, label: &str, bytes: &[u8]) -> Result<()> {
        let n = self.session.snapshots.len();
        let file = format!("snapshots/{n:02}-{label}.txt");
        self.store.write_rel(self.session.id, &file, bytes)?;
        let hash = format!("{:x}", md5_like(bytes));
        self.session.snapshots.push(TuneSnapshot { label: format!("{n:02}-{label}"), at: Utc::now(), file, hash });
        self.touch()
    }

    pub fn set_fc_tune(&mut self, tune: Option<Tune>, firmware: Option<Firmware>) -> Result<()> {
        self.session.fc_tune = tune;
        if firmware.is_some() {
            self.session.firmware = firmware;
        }
        self.touch()
    }

    pub fn set_report(&mut self, rel: String) -> Result<()> {
        self.session.report_file = Some(rel);
        self.touch()
    }

    pub fn set_notes(&mut self, notes: String) -> Result<()> {
        self.session.notes = notes;
        self.touch()
    }
}

/// Cheap content hash (FNV-1a 64) — enough to detect a changed snapshot.
fn md5_like(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let d = tempfile::tempdir().unwrap();
        let s = SessionStore::new(d.path());
        (d, s)
    }

    fn fake_log(fs: f64, dur: f64, raw: bool) -> (FlightLog, AnalysisBundle) {
        let n = (fs * dur) as usize;
        let mut axes: [AxisSeries; 3] = Default::default();
        for a in axes.iter_mut() {
            a.setpoint = vec![0.0; n];
            a.gyro_filt = vec![0.0; n];
            a.gyro_raw = if raw { Some(vec![0.0; n]) } else { None };
        }
        let log = FlightLog {
            id: LogId(format!("log{fs}{dur}{raw}")),
            firmware: Firmware::Betaflight { version: "4.5.1".into(), api: (0, 0) },
            fs_hz: fs,
            t: (0..n).map(|i| i as f32 / fs as f32).collect(),
            axes,
            motors: vec![],
            throttle: vec![0.45; n],
            erpm: None,
            gaps: vec![],
            meta: LogMeta { duration_s: dur, ..Default::default() },
            tune_at_log: Tune::Bf(BfTune::default()),
        };
        let bundle = AnalysisBundle {
            log: log.id.clone(),
            quality: LogQuality {
                fs_hz: fs,
                duration_s: dur,
                has_gyro_raw: raw,
                has_pid_terms: true,
                hover_seconds: dur,
                hover_throttle_pct: 45.0,
                airborne_range_s: None,
                motor_saturation_pct: 0.0,
                max_setpoint_per_axis: [300.0, 300.0, 300.0],
                step_segments_per_axis: [10, 10, 5],
                gap_seconds: 0.0,
            },
            steps: vec![],
            spectra: vec![],
            spectrograms: vec![],
            peaks: vec![],
        };
        (log, bundle)
    }

    #[test]
    fn offline_session_skips_fc_steps_and_enforces_guards() {
        let (_d, store) = store();
        let mut e = SessionEngine::create(store, "test".into(), Mode::Offline).unwrap();
        assert_eq!(e.session.current, Step::FlightA);
        assert_eq!(e.session.status[&Step::Connect], StepStatus::Skipped);
        // cannot advance before the flight is ticked
        assert!(matches!(e.next(None), Err(SessionError::GuardFailed(id, _)) if id == "flight_done"));
        e.mark_flight_done(Flight::A, true).unwrap();
        assert_eq!(e.next(None).unwrap(), Step::ImportA);
        // import a bad (1 kHz, no raw gyro) log → guards fail
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"H Product:Blackbox flight data recorder by Nicholas Sherlock\n").unwrap();
        let (log, bundle) = fake_log(1000.0, 60.0, false);
        e.attach_log(Flight::A, tmp.path(), 0, &log, &bundle).unwrap();
        let g = e.guards(Step::ImportA, None);
        assert!(g.iter().find(|g| g.id == "imported").unwrap().satisfied());
        assert!(!g.iter().find(|g| g.id == "log_rate").unwrap().satisfied());
        assert!(!g.iter().find(|g| g.id == "raw_gyro").unwrap().satisfied());
        assert!(e.next(None).is_err());
        // override both with a reason
        e.override_guard("log_rate", "customer can only log at 1 kHz").unwrap();
        e.override_guard("raw_gyro", "BF 4.3 without GYRO_SCALED, accept limited filter analysis").unwrap();
        assert!(e.override_guard("imported", "nope").is_err());
        assert_eq!(e.next(None).unwrap(), Step::FilterAnalysis);
        assert_eq!(e.next(None).unwrap(), Step::ApplyFilters);
        // no recommendations accepted → apply passes as explicit skip
        assert_eq!(e.next(None).unwrap(), Step::FlightB);
        assert!(e.session.overrides.len() == 2);
    }

    #[test]
    fn persists_and_resumes() {
        let (_d, store) = store();
        let id;
        {
            let mut e = SessionEngine::create(store.clone(), "resume".into(), Mode::Offline).unwrap();
            e.mark_flight_done(Flight::A, true).unwrap();
            e.next(None).unwrap();
            id = e.session.id;
        }
        let e = SessionEngine::open(store.clone(), id).unwrap();
        assert_eq!(e.session.current, Step::ImportA);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn online_needs_fc() {
        let (_d, store) = store();
        let mut e = SessionEngine::create(store, "online".into(), Mode::Online).unwrap();
        assert_eq!(e.session.current, Step::Connect);
        assert!(e.next(None).is_err());
        let fc = FcStatus {
            connected: true,
            firmware: Some(Firmware::Betaflight { version: "4.5.1".into(), api: (1, 46) }),
            tune: Some(Tune::Bf(BfTune::default())),
            snapshot_taken: true,
            ..Default::default()
        };
        assert_eq!(e.next(Some(&fc)).unwrap(), Step::Preflight);
        let g = e.guards(Step::Preflight, Some(&fc));
        assert!(g.iter().any(|g| !g.satisfied()));
        let fc2 = FcStatus { log_rate_hz: Some(2000.0), raw_gyro_logging_enabled: Some(true), storage_free_bytes: Some(16 << 20), ..fc.clone() };
        assert_eq!(e.next(Some(&fc2)).unwrap(), Step::FlightA);
        assert_eq!(e.back().unwrap(), Step::Preflight);
    }

    #[test]
    fn tune_mismatch_detected_after_apply() {
        let (_d, store) = store();
        let mut e = SessionEngine::create(store, "x".into(), Mode::Offline).unwrap();
        let mut rec = Recommendation {
            id: Uuid::new_v4(),
            param: ParamRef::Bf("d_roll".into()),
            old: ParamValue::U8(30),
            new: ParamValue::U8(40),
            reason: "t".into(),
            evidence: vec![],
            confidence: Confidence::High,
            requires_reboot: false,
            accepted: true,
        };
        rec.accepted = true;
        e.set_recs(ApplyPhase::Filters, vec![rec]).unwrap();
        e.record_apply(ApplyPhase::Filters, true, "cli", None).unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (log, bundle) = fake_log(2000.0, 60.0, true); // default tune: d_roll = 30, not 40
        e.attach_log(Flight::B, tmp.path(), 0, &log, &bundle).unwrap();
        let g = e.guards(Step::ImportB, None);
        let tm = g.iter().find(|g| g.id == "tune_match").unwrap();
        assert!(!tm.satisfied(), "{:?}", tm.outcome);
    }
}
