//! Online mode: flight-controller commands over a `Box<dyn FlightController>`
//! (Betaflight MSP or ArduPilot MAVLink).

use crate::wizard::{ImportResult, ReportImage};
use crate::AppState;
use domain::fc::{ApplyResult, FcError, FcKind, FlightController, PreflightFix};
use domain::*;
use fc_msp::{MspClient, PortInfo};
use serde::{Deserialize, Serialize};
use session::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

type R<T> = std::result::Result<T, String>;
pub type Client = Box<dyn FlightController>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConnectKind {
    #[default]
    Auto,
    Msp,
    Mavlink,
}

fn with_client<T>(state: &State<'_, AppState>, f: impl FnOnce(&mut dyn FlightController) -> std::result::Result<T, FcError>) -> R<T> {
    let mut g = state.client.lock().unwrap();
    let c = g.as_mut().ok_or_else(|| "flight controller not connected".to_string())?;
    f(c.as_mut()).map_err(|e| e.to_string())
}

/// Take the client out of the state for a blocking operation, then put it back.
async fn with_client_blocking<T: Send + 'static>(state: &State<'_, AppState>, f: impl FnOnce(&mut dyn FlightController) -> std::result::Result<T, FcError> + Send + 'static) -> R<T> {
    let mut client = state.client.lock().unwrap().take().ok_or("not connected")?;
    let (client, r) = tauri::async_runtime::spawn_blocking(move || {
        let r = f(client.as_mut());
        (client, r)
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.client.lock().unwrap() = Some(client);
    r.map_err(|e| e.to_string())
}

/// Refresh the shared FcStatus from the live FC (cheap poll).
fn refresh_status(state: &State<'_, AppState>) -> R<FcStatus> {
    let mut g = state.client.lock().unwrap();
    let Some(c) = g.as_mut() else {
        let s = FcStatus::default();
        *state.fc.lock().unwrap() = s.clone();
        return Ok(s);
    };
    let mut st = state.fc.lock().unwrap().clone();
    match c.poll() {
        Ok(s) => {
            st.connected = s.connected;
            st.armed = s.armed;
            st.heartbeat_age_s = s.heartbeat_age_s;
            if !s.connected {
                st.heartbeat_age_s += 1.0;
            }
        }
        Err(e) => {
            st.heartbeat_age_s += 1.0;
            if st.heartbeat_age_s > 5.0 {
                *g = None;
                st = FcStatus::default();
                *state.fc.lock().unwrap() = st.clone();
                return Err(format!("connection lost: {e}"));
            }
        }
    }
    *state.fc.lock().unwrap() = st.clone();
    Ok(st)
}

#[tauri::command]
pub fn fc_ports() -> Vec<PortInfo> {
    fc_msp::list_ports()
}

/// Open `port` as MSP or MAVLink. `auto` tries MAVLink heartbeat first (3 s), then MSP.
fn open_client(port: &str, kind: ConnectKind) -> std::result::Result<Client, FcError> {
    let try_mav = || -> std::result::Result<Client, FcError> { Ok(Box::new(fc_mavlink::MavClient::open(port, Duration::from_secs(3))?)) };
    let try_msp = || -> std::result::Result<Client, FcError> { Ok(Box::new(MspClient::open(port)?)) };
    match kind {
        ConnectKind::Msp => try_msp(),
        ConnectKind::Mavlink => try_mav(),
        ConnectKind::Auto => {
            if port.starts_with("tcp:") {
                return try_mav();
            }
            match try_msp() {
                Ok(c) => Ok(c),
                Err(msp_err) => try_mav().map_err(|mav_err| FcError::Other(format!("MSP: {msp_err}; MAVLink: {mav_err}"))),
            }
        }
    }
}

#[tauri::command]
pub async fn fc_connect(state: State<'_, AppState>, port: String, kind: Option<ConnectKind>) -> R<FcStatus> {
    let p = port.clone();
    let kind = kind.unwrap_or_default();
    let (client, mut status) = tauri::async_runtime::spawn_blocking(move || -> R<(Client, FcStatus)> {
        let mut c = open_client(&p, kind).map_err(|e| format!("{p}: {e}"))?;
        let s = c.full_status().map_err(|e| e.to_string())?;
        Ok((c, s))
    })
    .await
    .map_err(|e| e.to_string())??;
    // Automatic snapshot as the "00-" backup: MSP struct dump (BF) / full parameter list (AP).
    let snapshot: Option<(String, Vec<u8>)> = {
        let mut c = client;
        let snap = match c.kind() {
            FcKind::Msp => None, // BF: the cheap MSP snapshot is taken by the `diff all` backup step
            FcKind::Mavlink => c.backup().ok().map(|b| (format!("{}.{}", b.label, b.ext), b.bytes)),
        };
        *state.client.lock().unwrap() = Some(c);
        snap
    };
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.set_fc_tune(status.tune.clone(), status.firmware.clone()).map_err(|e| e.to_string())?;
        if let Some((label, bytes)) = snapshot {
            if !e.session.snapshots.iter().any(|s| s.label.starts_with("00-")) {
                e.add_snapshot(&label, &bytes).map_err(|e| e.to_string())?;
            }
            status.snapshot_taken = true;
        } else {
            status.snapshot_taken = e.session.snapshots.iter().any(|s| s.label.starts_with("00-"));
        }
    }
    *state.fc.lock().unwrap() = status.clone();
    Ok(status)
}

#[tauri::command]
pub fn fc_disconnect(state: State<'_, AppState>) -> R<FcStatus> {
    *state.client.lock().unwrap() = None;
    let s = FcStatus::default();
    *state.fc.lock().unwrap() = s.clone();
    Ok(s)
}

#[tauri::command]
pub fn fc_poll(state: State<'_, AppState>) -> R<FcStatus> {
    refresh_status(&state)
}

/// Re-read tune + logging config (after a fix or a reboot).
#[tauri::command]
pub fn fc_refresh(state: State<'_, AppState>) -> R<FcStatus> {
    let mut st = with_client(&state, |c| c.full_status())?;
    st.snapshot_taken = state.fc.lock().unwrap().snapshot_taken;
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.set_fc_tune(st.tune.clone(), st.firmware.clone()).map_err(|e| e.to_string())?;
    }
    *state.fc.lock().unwrap() = st.clone();
    Ok(st)
}

/// Full backup: BF `diff all` (reboots + reconnects), AP full `.param` list.
#[tauri::command]
pub async fn fc_backup_cli(state: State<'_, AppState>) -> R<FcStatus> {
    let b = with_client_blocking(&state, |c| c.backup()).await?;
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.add_snapshot(&format!("{}.{}", b.label, b.ext), &b.bytes).map_err(|e| e.to_string())?;
    }
    let mut st = fc_refresh(state.clone())?;
    st.snapshot_taken = true;
    *state.fc.lock().unwrap() = st.clone();
    Ok(st)
}

/// BF: blackbox rate ≥ 2 kHz (+ GYRO_SCALED on ≤ 4.3). AP: LOG_BITMASK bits 0/12/19 + batch sampler (reboots if needed).
#[tauri::command]
pub async fn fc_preflight_fix(state: State<'_, AppState>) -> R<FcStatus> {
    let r = with_client_blocking(&state, |c| c.preflight_fix(PreflightFix::Logging)).await?;
    if !r.verified {
        let bad: Vec<String> = r.outcomes.iter().filter(|o| !o.ok).map(|o| format!("{} → {} (read back {:?})", o.param, o.wanted, o.read_back)).collect();
        return Err(format!("logging fix not verified: {}", bad.join(", ")));
    }
    fc_refresh(state)
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

#[derive(Serialize)]
pub struct FcLogEntry {
    pub id: u32,
    pub size: u64,
    pub time_utc: Option<u64>,
}

#[tauri::command]
pub async fn fc_list_logs(state: State<'_, AppState>) -> R<Vec<FcLogEntry>> {
    let l = with_client_blocking(&state, |c| c.list_logs()).await?;
    Ok(l.into_iter().map(|e| FcLogEntry { id: e.id, size: e.size, time_utc: e.time_utc }).collect())
}

/// Download a log (BF: the flash; AP: `log_id` or the latest) and import it as flight `which`.
#[tauri::command]
pub async fn fc_download_import(app: AppHandle, state: State<'_, AppState>, which: Flight, log_id: Option<u32>) -> R<ImportResult> {
    let session_id = state.engine.lock().unwrap().as_ref().map(|e| e.session.id).ok_or("no session")?;
    let store = state.store.lock().unwrap().clone().ok_or("no store")?;
    let kind = state.fc.lock().unwrap().kind;
    let app2 = app.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let bytes = with_client_blocking(&state, move |c| {
        let mut progress = |d: u64, t: u64| {
            let _ = app2.emit("fc://progress", Progress { done: d, total: t });
        };
        c.download_log(log_id, &mut progress, &cancel)
    })
    .await?;
    let ext = match kind {
        Some(FcKind::Mavlink) => "bin",
        _ => "bbl",
    };
    let rel = format!("logs/download_{}.{ext}", format!("{which:?}").to_lowercase());
    store.write_rel(session_id, &rel, &bytes).map_err(|e| e.to_string())?;
    let path = store.abs(session_id, &rel).display().to_string();
    // Pick the last session in the dump (most recent flight).
    let sessions = log_ingest::list_sessions(&bytes);
    let idx = sessions.iter().rev().find(|s| s.error.is_none()).map(|s| s.index).unwrap_or(0);
    crate::wizard::flight_import(state, which, path, idx).await
}

#[derive(Serialize)]
pub struct FcApplyResult {
    pub result: ApplyResult,
    pub snapshot: SessionSnapshot,
}

/// Write the accepted recommendations of `phase` and verify by read-back
/// (the backend reboots + reconnects itself when a parameter needs it).
#[tauri::command]
pub async fn fc_apply(state: State<'_, AppState>, phase: ApplyPhase) -> R<FcApplyResult> {
    let recs: Vec<Recommendation> = state.engine.lock().unwrap().as_ref().ok_or("no session")?.session.recs(phase).clone();
    let kind = state.fc.lock().unwrap().kind;
    let result = with_client_blocking(&state, move |c| c.apply(&recs)).await?;
    let method = match kind {
        Some(FcKind::Mavlink) => "mavlink",
        _ => "msp",
    };
    let notes = serde_json::to_string(&result.outcomes).ok();
    let snapshot = {
        let fc = state.fc.lock().unwrap().clone();
        let mut g = state.engine.lock().unwrap();
        let e = g.as_mut().ok_or("no session")?;
        e.record_apply(phase, result.verified, method, notes).map_err(|e| e.to_string())?;
        e.snapshot(Some(&fc))
    };
    let _ = fc_refresh(state.clone());
    Ok(FcApplyResult { result, snapshot })
}

/// Text the pilot can apply by hand (BF CLI `set` lines / AP `NAME,VALUE`).
#[tauri::command]
pub fn fc_export_text(state: State<'_, AppState>, phase: ApplyPhase) -> R<String> {
    let g = state.engine.lock().unwrap();
    let e = g.as_ref().ok_or("no session")?;
    let fw = state.fc.lock().unwrap().firmware.clone().or_else(|| e.session.firmware.clone());
    Ok(domain::fc::export_text_for(fw.as_ref(), e.session.recs(phase)))
}

#[allow(dead_code)]
fn _unused(_: ReportImage, _: Arc<()>) {}
