//! Online mode: flight-controller commands backed by `fc-msp`.

use crate::wizard::{ImportResult, ReportImage};
use crate::AppState;
use domain::*;
use fc_msp::layouts::ApiVersion;
use fc_msp::{ApplyResult, MspClient, PortInfo};
use serde::Serialize;
use session::*;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type R<T> = std::result::Result<T, String>;

fn with_client<T>(state: &State<'_, AppState>, f: impl FnOnce(&mut MspClient) -> std::result::Result<T, fc_msp::MspError>) -> R<T> {
    let mut g = state.client.lock().unwrap();
    let c = g.as_mut().ok_or_else(|| "flight controller not connected".to_string())?;
    f(c).map_err(|e| e.to_string())
}

/// Refresh the shared FcStatus from the live FC.
fn refresh_status(state: &State<'_, AppState>) -> R<FcStatus> {
    let mut g = state.client.lock().unwrap();
    let Some(c) = g.as_mut() else {
        let s = FcStatus::default();
        *state.fc.lock().unwrap() = s.clone();
        return Ok(s);
    };
    let mut st = state.fc.lock().unwrap().clone();
    match c.status() {
        Ok(s) => {
            st.connected = true;
            st.armed = s.armed();
            st.heartbeat_age_s = 0.0;
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

/// Full read after connect: identity, tune, logging config, storage.
fn full_status(c: &mut MspClient) -> std::result::Result<(FcStatus, BfTune), fc_msp::MspError> {
    let tune = c.read_tune()?;
    let st = c.status()?;
    let api = c.api();
    let log_rate = c.blackbox_rate_hz()?;
    let debug_mode = tune.get_raw("debug_mode").and_then(|v| v.parse::<u8>().ok());
    // gyroUnfilt is logged natively from BF 4.4 (API 1.45); older builds need debug_mode 6 = GYRO_SCALED.
    let raw_ok = api.at_least(1, 45) || debug_mode == Some(6);
    let storage = match c.dataflash_summary() {
        Ok(d) if d.supported => Some(d.total_size.saturating_sub(d.used_size) as u64),
        _ => c.sdcard_summary()?.filter(|s| s.supported).map(|s| s.free_kb as u64 * 1024),
    };
    Ok((
        FcStatus {
            connected: true,
            port: Some(c.port.clone()),
            firmware: Some(c.firmware()),
            armed: st.armed(),
            heartbeat_age_s: 0.0,
            tune: Some(Tune::Bf(tune.clone())),
            log_rate_hz: log_rate,
            debug_mode: debug_mode.map(|d| if d == 6 { "GYRO_SCALED".to_string() } else { d.to_string() }),
            storage_free_bytes: storage,
            pid_logging_enabled: Some(true),
            raw_gyro_logging_enabled: Some(raw_ok),
            snapshot_taken: false,
        },
        tune,
    ))
}

#[tauri::command]
pub fn fc_ports() -> Vec<PortInfo> {
    fc_msp::list_ports()
}

#[tauri::command]
pub async fn fc_connect(state: State<'_, AppState>, port: String) -> R<FcStatus> {
    let p = port.clone();
    let (client, mut status, tune) = tauri::async_runtime::spawn_blocking(move || -> R<(MspClient, FcStatus, BfTune)> {
        let mut c = MspClient::open(&p).map_err(|e| format!("{p}: {e}"))?;
        let (s, t) = full_status(&mut c).map_err(|e| e.to_string())?;
        Ok((c, s, t))
    })
    .await
    .map_err(|e| e.to_string())??;
    // Automatic MSP snapshot as the "00-" backup.
    let snapshot_json = {
        let mut c = client;
        let snap = c.snapshot().map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(&snap).map_err(|e| e.to_string())?;
        *state.client.lock().unwrap() = Some(c);
        json
    };
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.set_fc_tune(Some(Tune::Bf(tune)), status.firmware.clone()).map_err(|e| e.to_string())?;
        if !e.session.snapshots.iter().any(|s| s.label.starts_with("00-")) {
            e.add_snapshot("msp-snapshot", &snapshot_json).map_err(|e| e.to_string())?;
        }
        status.snapshot_taken = true;
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
    let (mut st, tune) = with_client(&state, full_status)?;
    st.snapshot_taken = state.fc.lock().unwrap().snapshot_taken;
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.set_fc_tune(Some(Tune::Bf(tune)), st.firmware.clone()).map_err(|e| e.to_string())?;
    }
    *state.fc.lock().unwrap() = st.clone();
    Ok(st)
}

/// `diff all` backup through the CLI. Reboots the FC and reconnects.
#[tauri::command]
pub async fn fc_backup_cli(state: State<'_, AppState>) -> R<FcStatus> {
    let port = state.fc.lock().unwrap().port.clone().ok_or("not connected")?;
    let mut client = state.client.lock().unwrap().take().ok_or("not connected")?;
    let diff = tauri::async_runtime::spawn_blocking(move || -> R<String> {
        let d = client.cli_diff_all().map_err(|e| e.to_string())?;
        Ok(d)
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(e) = state.engine.lock().unwrap().as_mut() {
        e.add_snapshot("diff-all", diff.as_bytes()).map_err(|e| e.to_string())?;
    }
    let p = port.clone();
    let c = tauri::async_runtime::spawn_blocking(move || fc_msp::client::reconnect(&p, std::time::Duration::from_secs(15)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("reconnect after backup: {e}"))?;
    *state.client.lock().unwrap() = Some(c);
    let mut st = fc_refresh(state.clone())?;
    st.snapshot_taken = true;
    *state.fc.lock().unwrap() = st.clone();
    Ok(st)
}

/// Set blackbox rate ≥ 2 kHz and (on BF < 4.4) debug_mode = GYRO_SCALED.
#[tauri::command]
pub fn fc_preflight_fix(state: State<'_, AppState>) -> R<FcStatus> {
    with_client(&state, |c| {
        let api: ApiVersion = c.api();
        let st = c.status()?;
        if st.armed() {
            return Err(fc_msp::MspError::Refused("armed".into()));
        }
        let loop_hz = if st.cycle_time_us > 0 { 1e6 / st.cycle_time_us as f64 } else { 8000.0 };
        if let Some(mut bb) = c.read_blackbox()? {
            if api.at_least(1, 44) {
                // largest divisor that still gives ≥ 2 kHz
                let mut div = 0u8;
                while div < 4 && loop_hz / (1u32 << (div + 1)) as f64 >= 2000.0 {
                    div += 1;
                }
                bb.sample_rate = div;
            } else {
                bb.rate_num = 1;
                bb.rate_denom = ((loop_hz / 2000.0).floor() as u8).max(1);
            }
            if bb.device == 0 {
                bb.device = 1; // flash
            }
            c.write_blackbox(&bb)?;
        }
        if !api.at_least(1, 45) {
            if let Some(mut ac) = c.read_advanced_config()? {
                ac.debug_mode = 6; // GYRO_SCALED
                c.write_advanced_config(&ac)?;
            }
        }
        c.eeprom_write()?;
        Ok(())
    })?;
    fc_refresh(state)
}

#[derive(Serialize, Clone)]
struct Progress {
    done: u32,
    total: u32,
}

/// Download the flash and import it as flight `which`.
#[tauri::command]
pub async fn fc_download_import(app: AppHandle, state: State<'_, AppState>, which: Flight) -> R<ImportResult> {
    let session_id = state.engine.lock().unwrap().as_ref().map(|e| e.session.id).ok_or("no session")?;
    let store = state.store.lock().unwrap().clone().ok_or("no store")?;
    let mut client = state.client.lock().unwrap().take().ok_or("not connected")?;
    let app2 = app.clone();
    let (client, bytes) = tauri::async_runtime::spawn_blocking(move || {
        let r = client.dataflash_download(|d, t| {
            let _ = app2.emit("fc://progress", Progress { done: d, total: t });
        });
        (client, r)
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.client.lock().unwrap() = Some(client);
    let bytes = bytes.map_err(|e| e.to_string())?;
    let rel = format!("logs/flash_{}.bbl", format!("{which:?}").to_lowercase());
    store.write_rel(session_id, &rel, &bytes).map_err(|e| e.to_string())?;
    let path = store.abs(session_id, &rel).display().to_string();
    // Pick the last session in the dump (most recent flight).
    let sessions = bbl_ingest::list_sessions(&bytes);
    let idx = sessions.iter().rev().find(|s| s.error.is_none()).map(|s| s.index).unwrap_or(0);
    crate::wizard::flight_import(state, which, path, idx).await
}

#[derive(Serialize)]
pub struct FcApplyResult {
    pub result: ApplyResult,
    pub snapshot: SessionSnapshot,
}

/// Write the accepted recommendations of `phase` and verify by read-back.
#[tauri::command]
pub async fn fc_apply(state: State<'_, AppState>, phase: ApplyPhase) -> R<FcApplyResult> {
    let recs: Vec<Recommendation> = state.engine.lock().unwrap().as_ref().ok_or("no session")?.session.recs(phase).clone();
    let port = state.fc.lock().unwrap().port.clone().ok_or("not connected")?;
    let mut client = state.client.lock().unwrap().take().ok_or("not connected")?;
    let (client, result) = tauri::async_runtime::spawn_blocking(move || {
        let r = client.apply(&recs);
        (client, r)
    })
    .await
    .map_err(|e| e.to_string())?;
    let result = match result {
        Ok(r) => r,
        Err(e) => {
            *state.client.lock().unwrap() = Some(client);
            return Err(e.to_string());
        }
    };
    let client = if result.rebooted {
        let p = port.clone();
        tauri::async_runtime::spawn_blocking(move || fc_msp::client::reconnect(&p, std::time::Duration::from_secs(15)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("reconnect after save: {e}"))?
    } else {
        client
    };
    *state.client.lock().unwrap() = Some(client);
    let notes = serde_json::to_string(&result.outcomes).ok();
    let snapshot = {
        let fc = state.fc.lock().unwrap().clone();
        let mut g = state.engine.lock().unwrap();
        let e = g.as_mut().ok_or("no session")?;
        e.record_apply(phase, result.verified, "msp", notes).map_err(|e| e.to_string())?;
        e.snapshot(Some(&fc))
    };
    let _ = fc_refresh(state.clone());
    Ok(FcApplyResult { result, snapshot })
}

#[allow(dead_code)]
fn _unused(_: ReportImage, _: Arc<()>) {}
