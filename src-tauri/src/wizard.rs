use crate::AppState;
use domain::*;
use serde::{Deserialize, Serialize};
use session::*;
use std::path::Path;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

type R<T> = std::result::Result<T, String>;

fn store(state: &State<'_, AppState>) -> R<SessionStore> {
    state.store.lock().unwrap().clone().ok_or_else(|| "session store not ready".to_string())
}

fn with_engine<T>(state: &State<'_, AppState>, f: impl FnOnce(&mut SessionEngine, &FcStatus) -> R<T>) -> R<T> {
    let fc = state.fc.lock().unwrap().clone();
    let mut guard = state.engine.lock().unwrap();
    let e = guard.as_mut().ok_or_else(|| "no session open".to_string())?;
    f(e, &fc)
}

fn snap(e: &SessionEngine, fc: &FcStatus) -> SessionSnapshot {
    e.snapshot(if fc.connected { Some(fc) } else { None })
}

#[derive(Serialize)]
pub struct SessionSummary {
    pub id: Uuid,
    pub name: String,
    pub mode: Mode,
    pub current: Step,
    pub updated_at: String,
    pub firmware: Option<Firmware>,
    pub craft_name: Option<String>,
}

#[tauri::command]
pub fn session_list(state: State<'_, AppState>) -> R<Vec<SessionSummary>> {
    let list = store(&state)?.list().map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|s| SessionSummary {
            id: s.id,
            name: s.name.clone(),
            mode: s.mode,
            current: s.current,
            updated_at: s.updated_at.to_rfc3339(),
            firmware: s.firmware.clone(),
            craft_name: s.flights.values().find_map(|f| f.craft_name.clone()),
        })
        .collect())
}

#[tauri::command]
pub fn session_create(state: State<'_, AppState>, name: String, mode: Mode) -> R<SessionSnapshot> {
    let e = SessionEngine::create(store(&state)?, name, mode).map_err(|e| e.to_string())?;
    let fc = state.fc.lock().unwrap().clone();
    let s = snap(&e, &fc);
    *state.engine.lock().unwrap() = Some(e);
    Ok(s)
}

#[tauri::command]
pub fn session_open(state: State<'_, AppState>, id: Uuid) -> R<SessionSnapshot> {
    let e = SessionEngine::open(store(&state)?, id).map_err(|e| e.to_string())?;
    let fc = state.fc.lock().unwrap().clone();
    let s = snap(&e, &fc);
    *state.engine.lock().unwrap() = Some(e);
    Ok(s)
}

#[tauri::command]
pub fn session_delete(state: State<'_, AppState>, id: Uuid) -> R<()> {
    let mut g = state.engine.lock().unwrap();
    if g.as_ref().map(|e| e.session.id == id).unwrap_or(false) {
        *g = None;
    }
    store(&state)?.delete(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn session_snapshot(state: State<'_, AppState>) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| Ok(snap(e, fc)))
}

#[tauri::command]
pub fn wizard_next(state: State<'_, AppState>) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.next(if fc.connected { Some(fc) } else { None }).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[tauri::command]
pub fn wizard_back(state: State<'_, AppState>) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.back().map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[tauri::command]
pub fn wizard_goto(state: State<'_, AppState>, step: Step) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.goto(step).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[tauri::command]
pub fn wizard_override(state: State<'_, AppState>, guard_id: String, reason: String) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.override_guard(&guard_id, &reason).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[tauri::command]
pub fn flight_done(state: State<'_, AppState>, which: Flight, done: bool) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.mark_flight_done(which, done).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[derive(Serialize)]
pub struct ImportResult {
    pub snapshot: SessionSnapshot,
    pub bundle: AnalysisBundle,
    pub log_id: String,
}

/// Ingest + analyse + attach a log to a flight, and (re)compute the
/// recommendations that belong to that flight.
#[tauri::command]
pub async fn flight_import(state: State<'_, AppState>, which: Flight, path: String, session_index: usize) -> R<ImportResult> {
    let p = path.clone();
    let (log, bundle) = tauri::async_runtime::spawn_blocking(move || -> R<(FlightLog, AnalysisBundle)> {
        let bytes = std::fs::read(&p).map_err(|e| format!("read {p}: {e}"))?;
        let log = bbl_ingest::ingest(&bytes, session_index, &Default::default()).map_err(|e| e.to_string())?;
        let bundle = analysis::analyze(&log, &analysis::AnalysisOpts::default(), |_| {});
        Ok((log, bundle))
    })
    .await
    .map_err(|e| e.to_string())??;

    let log_id = log.id.0.clone();
    let snapshot = with_engine(&state, |e, fc| {
        e.attach_log(which, Path::new(&path), session_index, &log, &bundle).map_err(|e| e.to_string())?;
        let phase = match which {
            Flight::A => Some(recommend::Phase::Filters),
            Flight::B => Some(recommend::Phase::Pids),
            Flight::C => None,
        };
        if let Some(ph) = phase {
            let recs = recommend::recommend(&log.tune_at_log, &bundle, ph);
            let ap = if ph == recommend::Phase::Filters { ApplyPhase::Filters } else { ApplyPhase::Pids };
            e.set_recs(ap, recs).map_err(|e| e.to_string())?;
        }
        Ok(snap(e, fc))
    })?;
    state.logs.lock().unwrap().insert(log_id.clone(), Arc::new(log));
    Ok(ImportResult { snapshot, bundle, log_id })
}

#[tauri::command]
pub fn flight_bundle(state: State<'_, AppState>, which: Flight) -> R<Option<AnalysisBundle>> {
    with_engine(&state, |e, _| e.bundle(which).map_err(|e| e.to_string()))
}

#[derive(Deserialize)]
pub struct RecUpdate {
    pub phase: ApplyPhase,
    pub id: Uuid,
    pub accepted: bool,
    pub new_value: Option<ParamValue>,
}

#[tauri::command]
pub fn recs_set(state: State<'_, AppState>, update: RecUpdate) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.set_rec(update.phase, update.id, update.accepted, update.new_value).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

/// Offline mode: the tuner confirms the CLI/param text was applied by hand.
/// Online mode uses the `fc` layer, which records its own verified apply.
#[tauri::command]
pub fn apply_confirm(state: State<'_, AppState>, phase: ApplyPhase, method: String, notes: Option<String>) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.record_apply(phase, true, &method, notes).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[tauri::command]
pub fn fc_status(state: State<'_, AppState>) -> R<FcStatus> {
    Ok(state.fc.lock().unwrap().clone())
}

#[tauri::command]
pub fn session_notes(state: State<'_, AppState>, notes: String) -> R<SessionSnapshot> {
    with_engine(&state, |e, fc| {
        e.set_notes(notes).map_err(|e| e.to_string())?;
        Ok(snap(e, fc))
    })
}

#[derive(Deserialize)]
pub struct ReportImage {
    pub title: String,
    /// PNG data URL captured from the chart canvas.
    pub data_url: String,
}

#[tauri::command]
pub fn report_export(state: State<'_, AppState>, images: Vec<ReportImage>) -> R<String> {
    with_engine(&state, |e, _| {
        let bundles = [Flight::A, Flight::B, Flight::C]
            .iter()
            .filter_map(|f| e.bundle(*f).ok().flatten().map(|b| (*f, b)))
            .collect::<Vec<_>>();
        let html = crate::report::render(&e.session, &bundles, &images);
        let rel = "report/report.html".to_string();
        e.store.write_rel(e.session.id, &rel, html.as_bytes()).map_err(|e| e.to_string())?;
        e.set_report(rel.clone()).map_err(|e| e.to_string())?;
        Ok(e.store.abs(e.session.id, &rel).display().to_string())
    })
}
