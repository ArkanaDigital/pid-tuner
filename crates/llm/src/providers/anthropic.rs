//! Anthropic Messages API (`POST /v1/messages`, `anthropic-version: 2023-06-01`).
//! Tools: `{name, description, input_schema}`; tool call = `tool_use` content
//! block; results = ONE user message of `tool_result` blocks.

use super::{args_value, json_or_decode, post_json, status_error, u64_at};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, LlmError, Message, Part, ProviderCfg, Role,
    StopReason, Usage,
};
use serde_json::{json, Value};

pub struct Anthropic {
    pub cfg: ProviderCfg,
    pub client: reqwest::Client,
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::System => continue,
            Role::User => messages.push(json!({"role": "user", "content": m.text()})),
            Role::Assistant => {
                let content: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text } if !text.is_empty() => {
                            Some(json!({"type": "text", "text": text}))
                        }
                        Part::ToolUse { id, name, args } => {
                            Some(json!({"type": "tool_use", "id": id, "name": name, "input": args}))
                        }
                        _ => None,
                    })
                    .collect();
                if !content.is_empty() {
                    messages.push(json!({"role": "assistant", "content": content}));
                }
            }
            Role::Tool => {
                let content: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolResult { id, content, is_error, .. } => Some(json!({"type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error})),
                        _ => None,
                    })
                    .collect();
                messages.push(json!({"role": "user", "content": content}));
            }
        }
    }
    let mut body = json!({"model": req.model, "max_tokens": req.max_tokens, "system": req.system, "messages": messages});
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema})).collect());
    }
    body
}

pub fn parse(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> Result<ChatResponse, LlmError> {
    let v = json_or_decode(body)?;
    if status >= 400 {
        let msg = v
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .unwrap_or(body)
            .to_string();
        return Err(status_error(status, headers, msg));
    }
    let mut parts = Vec::new();
    for c in v
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
    {
        match c.get("type").and_then(|t| t.as_str()) {
            Some("text") => parts.push(Part::Text {
                text: c
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string(),
            }),
            Some("tool_use") => parts.push(Part::ToolUse {
                id: c
                    .get("id")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: c
                    .get("name")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string(),
                args: args_value(c.get("input").unwrap_or(&Value::Null)),
            }),
            _ => {}
        }
    }
    let stop = match v.get("stop_reason").and_then(|s| s.as_str()) {
        Some("tool_use") => StopReason::ToolUse,
        Some("end_turn") | Some("stop_sequence") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some(other) => StopReason::Other(other.to_string()),
        None => StopReason::EndTurn,
    };
    let usage = Usage {
        input: u64_at(&v, &["usage", "input_tokens"])
            + u64_at(&v, &["usage", "cache_creation_input_tokens"])
            + u64_at(&v, &["usage", "cache_read_input_tokens"]),
        output: u64_at(&v, &["usage", "output_tokens"]),
        cached: u64_at(&v, &["usage", "cache_read_input_tokens"]),
        reasoning: 0,
    };
    Ok(ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
            provider_state: None,
            provider: Some("anthropic".into()),
        },
        stop,
        usage,
        raw_id: v.get("id").and_then(|i| i.as_str()).map(String::from),
    })
}

#[async_trait::async_trait]
impl ChatProvider for Anthropic {
    fn name(&self) -> &'static str {
        "anthropic"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let url = format!("{}/v1/messages", self.cfg.base_url);
        let headers = vec![
            ("x-api-key", self.cfg.api_key.clone()),
            ("anthropic-version", "2023-06-01".to_string()),
        ];
        let (status, h, body) = post_json(
            &self.client,
            &url,
            headers,
            &build_body(req),
            self.cfg.timeout,
        )
        .await?;
        parse(status, &h, &body).map_err(|e| e.scrubbed(&[&self.cfg.api_key]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolDef;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "claude-sonnet-5".into(),
            system: "sys".into(),
            messages: vec![
                Message::user("hi"),
                Message {
                    role: Role::Assistant,
                    parts: vec![
                        Part::Text {
                            text: "checking".into(),
                        },
                        Part::ToolUse {
                            id: "toolu_1".into(),
                            name: "get_x".into(),
                            args: json!({"flight": "a"}),
                        },
                    ],
                    provider_state: None,
                    provider: Some("anthropic".into()),
                },
                Message {
                    role: Role::Tool,
                    parts: vec![
                        Part::ToolResult {
                            id: "toolu_1".into(),
                            name: "get_x".into(),
                            content: "{\"ok\":true}".into(),
                            is_error: false,
                        },
                        Part::ToolResult {
                            id: "toolu_2".into(),
                            name: "get_y".into(),
                            content: "boom".into(),
                            is_error: true,
                        },
                    ],
                    provider_state: None,
                    provider: None,
                },
            ],
            tools: vec![ToolDef::new(
                "get_x",
                "d",
                json!({"type":"object","properties":{"flight":{"type":"string"}},"required":["flight"],"additionalProperties":false}),
            )],
            max_tokens: 512,
        }
    }

    #[test]
    fn body_shape_matches_messages_api() {
        let b = build_body(&req());
        assert_eq!(b["system"], "sys");
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        assert!(b["tools"][0].get("parameters").is_none());
        assert_eq!(b["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(b["messages"][1]["content"][1]["input"]["flight"], "a");
        // both results in ONE user message
        assert_eq!(b["messages"][2]["role"], "user");
        assert_eq!(b["messages"][2]["content"].as_array().unwrap().len(), 2);
        assert_eq!(b["messages"][2]["content"][1]["is_error"], true);
        assert_eq!(b["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
        assert!(b.get("tool_choice").is_none());
    }

    #[test]
    fn parses_tool_use_text_usage_and_errors() {
        let h = reqwest::header::HeaderMap::new();
        let r = parse(200, &h, r#"{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"toolu_abc","name":"get_x","input":{"flight":"a"}}],"stop_reason":"tool_use","usage":{"input_tokens":120,"output_tokens":30,"cache_read_input_tokens":100,"cache_creation_input_tokens":0}}"#).unwrap();
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(r.message.tool_uses()[0].0, "toolu_abc");
        assert_eq!(r.message.text(), "Let me check.");
        assert_eq!(
            r.usage,
            Usage {
                input: 220,
                output: 30,
                cached: 100,
                reasoning: 0
            }
        );
        let r = parse(200, &h, r#"{"content":[{"type":"text","text":"done"}],"stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":2}}"#).unwrap();
        assert_eq!(r.stop, StopReason::EndTurn);
        let e = parse(401, &h, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#).unwrap_err();
        assert!(matches!(e, LlmError::Auth(m) if m.contains("invalid")));
        let mut h2 = reqwest::header::HeaderMap::new();
        h2.insert("retry-after", "3".parse().unwrap());
        assert!(
            matches!(parse(429, &h2, r#"{"error":{"message":"slow"}}"#).unwrap_err(), LlmError::RateLimited { retry_after: Some(d) } if d.as_secs() == 3)
        );
        assert!(matches!(
            parse(529, &h, r#"{"error":{"message":"overloaded"}}"#).unwrap_err(),
            LlmError::Retryable(_)
        ));
        assert!(matches!(
            parse(200, &h, "not json").unwrap_err(),
            LlmError::Decode(_)
        ));
    }
}
