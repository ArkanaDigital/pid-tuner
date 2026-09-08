//! Persistent conversation state per session.

use crate::{Message, Part, Role, Usage};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub args: serde_json::Value,
    pub ok: bool,
    /// First 500 chars of the result, for display.
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMeta {
    pub at: chrono::DateTime<chrono::Utc>,
    pub step: String,
    pub provider: String,
    pub model: String,
    pub user_text: String,
    pub reply: String,
    pub usage: Usage,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallRecord>,
    pub stopped_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub schema_version: u32,
    pub messages: Vec<Message>,
    pub usage_total: Usage,
    pub cost_total_usd: f64,
    pub turns: Vec<TurnMeta>,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            messages: Vec::new(),
            usage_total: Usage::default(),
            cost_total_usd: 0.0,
            turns: Vec::new(),
        }
    }
}

impl Transcript {
    /// The last `max_msgs` messages, starting at a user message, with orphan
    /// tool results dropped and foreign provider state stripped.
    pub fn context_window(&self, max_msgs: usize, provider: &str) -> Vec<Message> {
        let n = self.messages.len();
        let mut start = n.saturating_sub(max_msgs);
        while start < n && self.messages[start].role != Role::User {
            start += 1;
        }
        let mut out: Vec<Message> = Vec::new();
        let mut open_ids: Vec<String> = Vec::new();
        for m in &self.messages[start..] {
            let mut m = m.clone();
            if m.provider.as_deref() != Some(provider) {
                m.provider_state = None;
                m.provider = None;
            }
            match m.role {
                Role::Assistant => {
                    open_ids = m
                        .tool_uses()
                        .iter()
                        .map(|(id, _, _)| id.to_string())
                        .collect();
                    out.push(m);
                }
                Role::Tool => {
                    m.parts.retain(
                        |p| matches!(p, Part::ToolResult { id, .. } if open_ids.contains(id)),
                    );
                    if !m.parts.is_empty() {
                        out.push(m);
                    }
                    open_ids.clear();
                }
                _ => {
                    open_ids.clear();
                    out.push(m);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_msg(id: &str) -> Message {
        Message {
            role: Role::Tool,
            parts: vec![Part::ToolResult {
                id: id.into(),
                name: "t".into(),
                content: "{}".into(),
                is_error: false,
            }],
            provider_state: None,
            provider: None,
        }
    }
    fn tool_use(id: &str, prov: &str) -> Message {
        Message {
            role: Role::Assistant,
            parts: vec![Part::ToolUse {
                id: id.into(),
                name: "t".into(),
                args: serde_json::json!({}),
            }],
            provider_state: Some(serde_json::json!({"x":1})),
            provider: Some(prov.into()),
        }
    }

    #[test]
    fn window_starts_at_user_and_drops_orphans_and_foreign_state() {
        let mut t = Transcript::default();
        t.messages = vec![
            Message::user("a"),
            tool_use("1", "openai"),
            tool_msg("1"),
            Message::assistant_text("done"),
            Message::user("b"),
            tool_use("2", "openai"),
            tool_msg("2"),
            tool_msg("zzz"),
            Message::assistant_text("ok"),
        ];
        let w = t.context_window(6, "anthropic");
        assert_eq!(w[0].role, Role::User);
        assert_eq!(w[0].text(), "b");
        assert!(w.iter().all(|m| m.provider_state.is_none()));
        let tools: Vec<&Message> = w.iter().filter(|m| m.role == Role::Tool).collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].parts.len(), 1);
        let w2 = t.context_window(100, "openai");
        assert_eq!(w2.len(), 8, "orphan tool message dropped: {w2:?}");
        assert!(w2[1].provider_state.is_some());
    }
}
