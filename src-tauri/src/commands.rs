use crate::AppState;
use domain::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct LogSummary {
    pub id: String,
    pub path: String,
    pub firmware: Firmware,
    pub craft_name: Option<String>,
    pub fs_hz: f64,
    pub duration_s: f64,
    pub samples: usize,
    pub session_index: usize,
    pub session_count: usize,
    pub debug_mode: Option<String>,
    pub warnings: Vec<String>,
    pub has_gyro_raw: bool,
    pub tune: Tune,
}

fn summarize(path: &str, log: &FlightLog) -> LogSummary {
    LogSummary {
        id: log.id.0.clone(),
        path: path.to_string(),
        firmware: log.firmware.clone(),
        craft_name: log.meta.craft_name.clone(),
        fs_hz: log.fs_hz,
        duration_s: log.duration_s(),
        samples: log.len(),
        session_index: log.meta.session_index,
        session_count: log.meta.session_count,
        debug_mode: log.meta.debug_mode.clone(),
        warnings: log.meta.warnings.clone(),
        has_gyro_raw: log.axes.iter().all(|a| a.gyro_raw.is_some()),
        tune: log.tune_at_log.clone(),
    }
}

#[tauri::command]
pub async fn log_sessions(path: String) -> Result<Vec<log_ingest::SessionInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
        Ok(log_ingest::list_sessions(&bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn log_open(state: State<'_, AppState>, path: String, session: usize) -> Result<LogSummary, String> {
    let p = path.clone();
    let log = tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&p).map_err(|e| format!("read {p}: {e}"))?;
        log_ingest::ingest(&bytes, session).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let summary = summarize(&path, &log);
    state.logs.lock().unwrap().insert(log.id.0.clone(), Arc::new(log));
    Ok(summary)
}

fn get_log(state: &State<'_, AppState>, id: &str) -> Result<Arc<FlightLog>, String> {
    state.logs.lock().unwrap().get(id).cloned().ok_or_else(|| format!("log {id} not loaded"))
}

#[tauri::command]
pub async fn log_analyze(state: State<'_, AppState>, id: String, pid_analyzer: bool) -> Result<AnalysisBundle, String> {
    let log = get_log(&state, &id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut opts = analysis::AnalysisOpts::default();
        if pid_analyzer {
            opts.step = analysis::StepOpts::pid_analyzer();
        }
        analysis::analyze(&log, &opts, |_| {})
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct RecommendArgs {
    pub id: String,
    pub bundle: AnalysisBundle,
    pub phase: String,
}

#[tauri::command]
pub async fn log_recommend(state: State<'_, AppState>, args: RecommendArgs) -> Result<Vec<Recommendation>, String> {
    let log = get_log(&state, &args.id)?;
    let phase = if args.phase == "pids" { recommend::Phase::Pids } else { recommend::Phase::Filters };
    Ok(recommend::recommend_for_log(&log, &args.bundle, phase))
}

#[derive(Serialize)]
pub struct SeriesWindow {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

/// Min/max-decimated slice of a time series for plotting.
/// `series`: "setpoint" | "gyro_filt" | "gyro_raw" | "p" | "i" | "d" | "ff" | "throttle" | "motor0".."motor7"
#[tauri::command]
pub fn series_window(
    state: State<'_, AppState>,
    id: String,
    series: String,
    axis: usize,
    t0: f32,
    t1: f32,
    px_width: usize,
) -> Result<SeriesWindow, String> {
    let log = get_log(&state, &id)?;
    let ax = &log.axes[axis.min(2)];
    let y: &[f32] = match series.as_str() {
        "setpoint" => &ax.setpoint,
        "gyro_filt" => &ax.gyro_filt,
        "gyro_raw" => ax.gyro_raw.as_deref().ok_or("no gyro_raw")?,
        "p" => ax.p.as_deref().ok_or("no p")?,
        "i" => ax.i.as_deref().ok_or("no i")?,
        "d" => ax.d.as_deref().ok_or("no d")?,
        "ff" => ax.ff.as_deref().ok_or("no ff")?,
        "throttle" => &log.throttle,
        s if s.starts_with("motor") => {
            let k: usize = s[5..].parse().map_err(|_| "bad motor index")?;
            log.motors.get(k).ok_or("no such motor")?
        }
        _ => return Err(format!("unknown series {series}")),
    };
    let (i0, i1) = dsp::decimate::range_indices(&log.t, t0, t1);
    let (x, y) = dsp::decimate::minmax(&log.t[i0..i1], &y[i0..i1], px_width.max(10));
    Ok(SeriesWindow { x, y })
}

/// Dev helper: `PIDTUNER_OPEN=/path/to/log.bbl pnpm tauri dev` auto-loads a file.
#[tauri::command]
pub fn dev_autoload_path() -> Option<String> {
    std::env::var("PIDTUNER_OPEN").ok().filter(|s| !s.is_empty())
}
