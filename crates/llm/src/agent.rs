//! The tool-calling loop: budget check → provider call → run tools → repeat.

use crate::retry::{with_retry, RetryPolicy, Sleeper, TokioSleeper};
use crate::tools::truncate_result;
use crate::transcript::{ToolCallRecord, Transcript, TurnMeta};
use crate::{
    estimate_cost, ChatProvider, ChatRequest, LlmError, Message, ModelPrice, Part, Role,
    StopReason, ToolHost, Usage,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_rounds: u8,
    /// 0 = unlimited
    pub budget_tokens: u64,
    pub max_tokens: u32,
    pub price: Option<ModelPrice>,
    pub context_messages: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            max_rounds: 8,
            budget_tokens: 0,
            max_tokens: 2048,
            price: None,
            context_messages: 40,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    Thinking {
        round: u8,
    },
    ToolCall {
        round: u8,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        round: u8,
        name: String,
        ok: bool,
        chars: usize,
    },
    Usage {
        turn: Usage,
        session: Usage,
        cost_usd: f64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentTurn {
    pub reply: String,
    pub usage: Usage,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallRecord>,
    pub stopped_by: Option<String>,
}

pub struct Agent {
    pub provider: Box<dyn ChatProvider>,
    pub host: Arc<dyn ToolHost>,
    pub cfg: AgentConfig,
    pub retry: RetryPolicy,
    pub sleeper: Box<dyn Sleeper>,
}

impl Agent {
    pub fn new(provider: Box<dyn ChatProvider>, host: Arc<dyn ToolHost>, cfg: AgentConfig) -> Self {
        Self {
            provider,
            host,
            cfg,
            retry: RetryPolicy::default(),
            sleeper: Box::new(TokioSleeper),
        }
    }

    fn budget_check(&self, t: &Transcript) -> Result<(), LlmError> {
        if self.cfg.budget_tokens > 0 && t.usage_total.total() >= self.cfg.budget_tokens {
            return Err(LlmError::Budget {
                used: t.usage_total.total(),
                limit: self.cfg.budget_tokens,
            });
        }
        Ok(())
    }

    fn cost(&self, u: &Usage) -> f64 {
        self.cfg
            .price
            .as_ref()
            .map(|p| estimate_cost(u.input, u.output, u.cached, p))
            .unwrap_or(0.0)
    }

    /// One user turn, with up to `max_rounds` tool rounds. The transcript is
    /// updated in place (messages, usage, turns) even when the loop stops early.
    pub async fn run(
        &self,
        system: &str,
        transcript: &mut Transcript,
        step: &str,
        user_text: String,
        on_event: &(dyn Fn(AgentEvent) + Send + Sync),
    ) -> Result<AgentTurn, LlmError> {
        self.budget_check(transcript)?;
        transcript.messages.push(Message::user(user_text.clone()));
        let tools = self.host.defs();
        let mut turn_usage = Usage::default();
        let mut records: Vec<ToolCallRecord> = Vec::new();
        let mut stopped_by: Option<String> = None;
        let mut reply = String::new();
        let mut round: u8 = 0;
        loop {
            round += 1;
            let final_round = round > self.cfg.max_rounds;
            if final_round {
                stopped_by = Some("max_rounds".into());
            }
            on_event(AgentEvent::Thinking { round });
            let req = ChatRequest {
                model: self.cfg.model.clone(),
                system: system.to_string(),
                messages: transcript
                    .context_window(self.cfg.context_messages, self.provider.name()),
                tools: if final_round {
                    Vec::new()
                } else {
                    tools.clone()
                },
                max_tokens: self.cfg.max_tokens,
            };
            let provider = &self.provider;
            let resp = with_retry(&self.retry, self.sleeper.as_ref(), || {
                provider.complete(&req)
            })
            .await?;
            turn_usage += resp.usage;
            transcript.usage_total += resp.usage;
            let cost = self.cost(&resp.usage);
            transcript.cost_total_usd += cost;
            on_event(AgentEvent::Usage {
                turn: turn_usage,
                session: transcript.usage_total,
                cost_usd: self.cost(&transcript.usage_total),
            });
            let text = resp.message.text();
            let calls: Vec<(String, String, serde_json::Value)> = resp
                .message
                .tool_uses()
                .iter()
                .map(|(i, n, a)| (i.to_string(), n.to_string(), (*a).clone()))
                .collect();
            transcript.messages.push(resp.message.clone());
            if !text.is_empty() {
                reply = text;
            }
            if resp.stop == StopReason::MaxTokens {
                stopped_by = Some("max_tokens".into());
            }
            if calls.is_empty() || final_round {
                break;
            }
            // run every tool of this round, then ONE tool message with all results (order kept)
            let mut results: Vec<Part> = Vec::new();
            let known: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            for (id, name, args) in calls {
                on_event(AgentEvent::ToolCall {
                    round,
                    name: name.clone(),
                    args: args.clone(),
                });
                let outcome: Result<String, String> = if !args.is_object() {
                    Err("invalid arguments: expected a JSON object".into())
                } else if !known.contains(&name.as_str()) {
                    Err(format!("unknown tool `{name}`"))
                } else {
                    self.host.call(&name, args.clone()).await
                };
                let (content, ok) = match outcome {
                    Ok(s) => (truncate_result(s), true),
                    Err(e) => (e, false),
                };
                on_event(AgentEvent::ToolResult {
                    round,
                    name: name.clone(),
                    ok,
                    chars: content.len(),
                });
                records.push(ToolCallRecord {
                    name: name.clone(),
                    args,
                    ok,
                    preview: content.chars().take(500).collect(),
                });
                results.push(Part::ToolResult {
                    id,
                    name,
                    content,
                    is_error: !ok,
                });
            }
            transcript.messages.push(Message {
                role: Role::Tool,
                parts: results,
                provider_state: None,
                provider: None,
            });
            if self.cfg.budget_tokens > 0
                && transcript.usage_total.total() >= self.cfg.budget_tokens
            {
                stopped_by = Some("budget".into());
                if reply.is_empty() {
                    reply = format!("Batas token sesi tercapai ({} dari {}). Naikkan budget di Settings untuk melanjutkan.", transcript.usage_total.total(), self.cfg.budget_tokens);
                }
                break;
            }
        }
        let turn = AgentTurn {
            reply: reply.clone(),
            usage: turn_usage,
            cost_usd: self.cost(&turn_usage),
            tool_calls: records.clone(),
            stopped_by: stopped_by.clone(),
        };
        transcript.turns.push(TurnMeta {
            at: chrono::Utc::now(),
            step: step.to_string(),
            provider: self.provider.name().to_string(),
            model: self.cfg.model.clone(),
            user_text,
            reply,
            usage: turn_usage,
            cost_usd: turn.cost_usd,
            tool_calls: records,
            stopped_by,
        });
        Ok(turn)
    }
}
