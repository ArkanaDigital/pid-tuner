//! Tauri shell: commands exposed to the UI. Heavy work runs on blocking threads.

mod commands;
mod fc;
mod report;
mod wizard;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::Manager;

#[derive(Default)]
pub struct AppState {
    pub logs: Mutex<HashMap<String, Arc<domain::FlightLog>>>,
    pub bundles: Mutex<HashMap<String, Arc<domain::AnalysisBundle>>>,
    pub engine: Mutex<Option<session::SessionEngine>>,
    pub store: Mutex<Option<session::SessionStore>>,
    pub fc: Mutex<session::FcStatus>,
    pub client: Mutex<Option<fc::Client>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir").join("sessions");
            std::fs::create_dir_all(&dir).ok();
            *app.state::<AppState>().store.lock().unwrap() = Some(session::SessionStore::new(dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::log_sessions,
            commands::log_open,
            commands::log_analyze,
            commands::log_recommend,
            commands::series_window,
            commands::dev_autoload_path,
            wizard::session_list,
            wizard::session_create,
            wizard::session_open,
            wizard::session_delete,
            wizard::session_snapshot,
            wizard::wizard_next,
            wizard::wizard_back,
            wizard::wizard_goto,
            wizard::wizard_override,
            wizard::flight_done,
            wizard::flight_import,
            wizard::flight_bundle,
            wizard::recs_set,
            wizard::apply_confirm,
            wizard::fc_status,
            wizard::report_export,
            wizard::session_notes,
            wizard::wizard_pid_strategy,
            wizard::flight_protocol,
            wizard::save_text_file,
            fc::fc_ports,
            fc::fc_connect,
            fc::fc_disconnect,
            fc::fc_poll,
            fc::fc_refresh,
            fc::fc_backup_cli,
            fc::fc_preflight_fix,
            fc::fc_download_import,
            fc::fc_apply,
            fc::fc_list_logs,
            fc::fc_export_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
