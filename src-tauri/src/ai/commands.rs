//! `ai_*` commands: one agent turn, transcript access, budget.

use super::host::{AiHost, QuickCtx, Scope};
use crate::settings::current;
use crate::AppState;
use llm::{Agent, AgentConfig, AgentEvent, Transcript};
use serde::{Deserialize, Serialize};
use session::SessionSnapshot;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type R<T> = std::result::Result<T, String>;

#[derive(Deserialize)]
pub struct AiChatRequest {
    pub scope: Scope,
    pub text: String,
}

#[derive(Serialize)]
pub struct AiTurnView {
    pub reply: String,
    pub usage: llm::Usage,
    pub cost_usd: f64,
    pub session_usage: llm::Usage,
    pub session_cost_usd: f64,
    pub tool_calls: Vec<llm::ToolCallRecord>,
    pub stopped_by: Option<String>,
    pub provider: String,
    pub model: String,
    /// Present when a recommendation changed (session scope) so the UI refreshes.
    pub snapshot: Option<SessionSnapshot>,
    /// Quick-look recommendations after the turn (quick scope).
    pub quick_recs: Option<Vec<domain::Recommendation>>,
}

#[derive(Serialize)]
pub struct TranscriptView {
    pub turns: Vec<llm::TurnMeta>,
    pub usage_total: llm::Usage,
    pub cost_total_usd: f64,
}

#[derive(Serialize)]
pub struct BudgetView {
    pub used_tokens: u64,
    pub limit_tokens: u64,
    pub cost_usd: f64,
    pub provider: String,
    pub model: String,
    pub key_set: bool,
}

fn transcript_path(state: &AppState) -> Option<std::path::PathBuf> {
    let store = state.store.lock().unwrap().clone()?;
    let id = state
        .engine
        .lock()
        .unwrap()
        .as_ref()
        .map(|e| e.session.id)?;
    Some(store.abs(id, "ai/transcript.json"))
}

fn load_transcript(state: &AppState, scope: Scope) -> Transcript {
    match scope {
        Scope::Session => transcript_path(state)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default(),
        Scope::Quick => state
            .quick
            .lock()
            .unwrap()
            .as_ref()
            .map(|q| q.transcript.clone())
            .unwrap_or_default(),
    }
}

fn save_transcript(state: &AppState, scope: Scope, t: &Transcript) -> R<()> {
    match scope {
        Scope::Session => {
            let p = transcript_path(state).ok_or("no session open")?;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let tmp = p.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(t).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
        }
        Scope::Quick => {
            let mut q = state.quick.lock().unwrap();
            q.get_or_insert_with(QuickCtx::default).transcript = t.clone();
            Ok(())
        }
    }
}

fn prompt_ctx(state: &AppState, scope: Scope) -> llm::prompt::PromptCtx {
    let fc = state.fc.lock().unwrap().clone();
    let g = state.engine.lock().unwrap();
    match (scope, g.as_ref()) {
        (Scope::Session, Some(e)) => {
            let guards = e.guards(
                e.session.current,
                if fc.connected { Some(&fc) } else { None },
            );
            llm::prompt::PromptCtx {
                firmware: e
                    .session
                    .firmware
                    .as_ref()
                    .or(fc.firmware.as_ref())
                    .map(|f| format!("{f:?}")),
                mode: format!("{:?}", e.session.mode).to_lowercase(),
                current_step: format!("{:?}", e.session.current).to_lowercase(),
                step_title: e.session.current.title().into(),
                guards_failed: guards
                    .iter()
                    .filter(|g| !g.satisfied())
                    .map(|g| g.title.to_string())
                    .collect(),
                scope: "session (wizard)".into(),
            }
        }
        _ => {
            let q = state.quick.lock().unwrap();
            let id = q
                .as_ref()
                .and_then(|q| q.log_id.clone())
                .unwrap_or_default();
            let fw = state
                .logs
                .lock()
                .unwrap()
                .get(&id)
                .map(|l| format!("{:?}", l.firmware));
            llm::prompt::PromptCtx {
                firmware: fw,
                mode: "offline".into(),
                current_step: "quick_look".into(),
                step_title: "Quick look".into(),
                guards_failed: vec![],
                scope: "quick look (single log, no wizard)".into(),
            }
        }
    }
}

#[tauri::command]
pub async fn ai_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AiChatRequest,
) -> R<AiTurnView> {
    {
        let mut busy = state.ai_busy.lock().unwrap();
        if *busy {
            return Err("AI masih memproses permintaan sebelumnya".into());
        }
        *busy = true;
    }
    let result = ai_chat_inner(app, &state, req).await;
    *state.ai_busy.lock().unwrap() = false;
    result
}

async fn ai_chat_inner(
    app: AppHandle,
    state: &State<'_, AppState>,
    req: AiChatRequest,
) -> R<AiTurnView> {
    let s = current(state);
    let provider = s.provider;
    let key = s
        .key(provider)
        .ok_or_else(|| format!("Kunci API {} belum diisi — buka Settings", provider.title()))?
        .expose()
        .to_string();
    let model = s.model(provider);
    let cfg = llm::ProviderCfg::new(key.clone(), s.base_url(provider), model.clone());
    let prov = llm::make_provider(provider, cfg, state.http.clone()).map_err(|e| e.to_string())?;
    if req.scope == Scope::Session && state.engine.lock().unwrap().is_none() {
        return Err("no session open".into());
    }
    let host = Arc::new(AiHost::tauri(app.clone(), req.scope));
    let agent = Agent::new(
        prov,
        host,
        AgentConfig {
            model: model.clone(),
            max_rounds: s.max_tool_rounds,
            budget_tokens: s.token_budget_per_session,
            max_tokens: 2048,
            price: s.price_for(&model),
            context_messages: 40,
        },
    );
    let ctx = prompt_ctx(state, req.scope);
    let system = llm::prompt::system(&s.language, &ctx);
    let mut transcript = load_transcript(state, req.scope);
    let app2 = app.clone();
    let on_event = move |e: AgentEvent| {
        let _ = app2.emit("ai://progress", &e);
    };
    let outcome = agent
        .run(
            &system,
            &mut transcript,
            &ctx.current_step,
            req.text,
            &on_event,
        )
        .await;
    save_transcript(state, req.scope, &transcript)?;
    let turn = outcome.map_err(|e| llm::redact::scrub(&e.to_string(), &[&key]))?;
    let changed = turn
        .tool_calls
        .iter()
        .any(|c| c.ok && (c.name == "set_recommendation" || c.name == "add_recommendation"));
    let snapshot = if changed && req.scope == Scope::Session {
        let fc = state.fc.lock().unwrap().clone();
        state
            .engine
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.snapshot(if fc.connected { Some(&fc) } else { None }))
    } else {
        None
    };
    let quick_recs = if changed && req.scope == Scope::Quick {
        state.quick.lock().unwrap().as_ref().map(|q| {
            q.recs_filters
                .iter()
                .chain(q.recs_pids.iter())
                .cloned()
                .collect()
        })
    } else {
        None
    };
    Ok(AiTurnView {
        reply: turn.reply,
        usage: turn.usage,
        cost_usd: turn.cost_usd,
        session_usage: transcript.usage_total,
        session_cost_usd: transcript.cost_total_usd,
        tool_calls: turn.tool_calls,
        stopped_by: turn.stopped_by,
        provider: provider.name().into(),
        model,
        snapshot,
        quick_recs,
    })
}

#[tauri::command]
pub fn ai_transcript_get(state: State<'_, AppState>, scope: Scope) -> R<TranscriptView> {
    let t = load_transcript(&state, scope);
    Ok(TranscriptView {
        turns: t.turns,
        usage_total: t.usage_total,
        cost_total_usd: t.cost_total_usd,
    })
}

#[tauri::command]
pub fn ai_transcript_clear(state: State<'_, AppState>, scope: Scope) -> R<()> {
    match scope {
        Scope::Session => {
            if let Some(p) = transcript_path(&state) {
                let _ = std::fs::remove_file(p);
            }
        }
        Scope::Quick => {
            if let Some(q) = state.quick.lock().unwrap().as_mut() {
                q.transcript = Transcript::default();
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn ai_budget(state: State<'_, AppState>, scope: Scope) -> R<BudgetView> {
    let s = current(&state);
    let t = load_transcript(&state, scope);
    Ok(BudgetView {
        used_tokens: t.usage_total.total(),
        limit_tokens: s.token_budget_per_session,
        cost_usd: t.cost_total_usd,
        provider: s.provider.name().into(),
        model: s.model(s.provider),
        key_set: s.key(s.provider).is_some(),
    })
}
