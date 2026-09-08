//! App settings (LLM providers, keys, budget) persisted next to the sessions.

use crate::AppState;
use appconfig::{Provider, Settings, SettingsPatch, SettingsStore, SettingsView};
use serde::Serialize;
use tauri::State;

type R<T> = std::result::Result<T, String>;

/// Current settings (loaded once, cached in `AppState`).
pub fn current(state: &AppState) -> Settings {
    if let Some(s) = state.settings_cache.lock().unwrap().as_ref() {
        return s.clone();
    }
    let loaded = state
        .settings
        .lock()
        .unwrap()
        .as_ref()
        .map(|st| st.load())
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default();
    *state.settings_cache.lock().unwrap() = Some(loaded.clone());
    loaded
}

fn save(state: &AppState, s: &Settings) -> R<()> {
    if let Some(store) = state.settings.lock().unwrap().as_ref() {
        store.save(s).map_err(|e| e.to_string())?;
    }
    *state.settings_cache.lock().unwrap() = Some(s.clone());
    Ok(())
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> R<SettingsView> {
    Ok(current(&state).view())
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, patch: SettingsPatch) -> R<SettingsView> {
    let mut s = current(&state);
    s.apply(patch);
    save(&state, &s)?;
    Ok(s.view())
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub model: String,
    pub latency_ms: u64,
    pub usage: llm::Usage,
    pub message: String,
}

/// Tiny request to verify a key + model; errors come back scrubbed.
#[tauri::command]
pub async fn settings_test_provider(
    state: State<'_, AppState>,
    provider: Provider,
) -> R<TestResult> {
    let s = current(&state);
    let key = s
        .key(provider)
        .ok_or_else(|| format!("Kunci API {} belum diisi", provider.title()))?
        .expose()
        .to_string();
    let model = s.model(provider);
    let cfg = llm::ProviderCfg::new(key.clone(), s.base_url(provider), model.clone());
    let prov = llm::make_provider(provider, cfg, state.http.clone()).map_err(|e| e.to_string())?;
    let req = llm::ChatRequest {
        model: model.clone(),
        system: "Reply with the single word OK.".into(),
        messages: vec![llm::Message::user("ping")],
        tools: vec![],
        max_tokens: 8,
    };
    let t0 = std::time::Instant::now();
    match prov.complete(&req).await {
        Ok(r) => Ok(TestResult {
            ok: true,
            model,
            latency_ms: t0.elapsed().as_millis() as u64,
            usage: r.usage,
            message: r.message.text(),
        }),
        Err(e) => Ok(TestResult {
            ok: false,
            model,
            latency_ms: t0.elapsed().as_millis() as u64,
            usage: llm::Usage::default(),
            message: llm::redact::scrub(&e.to_string(), &[&key]),
        }),
    }
}

pub fn store_for(dir: &std::path::Path) -> SettingsStore {
    SettingsStore::new(dir.join("settings.json"))
}
