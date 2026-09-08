//! DeepSeek `POST /chat/completions` (OpenAI Chat-Completions shape).
//! `reasoning_content` from a tool-calling turn must be echoed back on the
//! assistant message; `finish_reason: insufficient_system_resource` is
//! retryable; HTTP 402 = out of balance.

use super::{args_value, json_or_decode, post_json, status_error, u64_at};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, LlmError, Message, Part, ProviderCfg, Role,
    StopReason, Usage,
};
use serde_json::{json, Value};

pub struct DeepSeek {
    pub cfg: ProviderCfg,
    pub client: reqwest::Client,
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = vec![json!({"role": "system", "content": req.system})];
    for m in &req.messages {
        match m.role {
            Role::System => continue,
            Role::User => messages.push(json!({"role": "user", "content": m.text()})),
            Role::Assistant => {
                let mut msg = json!({"role": "assistant", "content": m.text()});
                let calls: Vec<Value> = m.tool_uses().iter().map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}})).collect();
                if !calls.is_empty() {
                    msg["tool_calls"] = Value::Array(calls);
                }
                if let Some(Value::String(rc)) = m
                    .provider_state
                    .as_ref()
                    .and_then(|s| s.get("reasoning_content"))
                {
                    msg["reasoning_content"] = Value::String(rc.clone());
                }
                messages.push(msg);
            }
            Role::Tool => {
                for p in &m.parts {
                    if let Part::ToolResult {
                        id,
                        content,
                        is_error,
                        ..
                    } = p
                    {
                        let c = if *is_error {
                            format!("{{\"error\":{}}}", Value::String(content.clone()))
                        } else {
                            content.clone()
                        };
                        messages.push(json!({"role": "tool", "tool_call_id": id, "content": c}));
                    }
                }
            }
        }
    }
    let mut body = json!({"model": req.model, "messages": messages, "max_tokens": req.max_tokens});
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}})).collect());
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
    let choice = v.pointer("/choices/0").cloned().unwrap_or(Value::Null);
    let msg = choice.get("message").cloned().unwrap_or(Value::Null);
    let mut parts = Vec::new();
    if let Some(t) = msg.get("content").and_then(|c| c.as_str()) {
        if !t.is_empty() {
            parts.push(Part::Text {
                text: t.to_string(),
            });
        }
    }
    for tc in msg
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default()
    {
        parts.push(Part::ToolUse {
            id: tc
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or_default()
                .to_string(),
            name: tc
                .pointer("/function/name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
            args: args_value(tc.pointer("/function/arguments").unwrap_or(&Value::Null)),
        });
    }
    let stop = match choice.get("finish_reason").and_then(|f| f.as_str()) {
        Some("tool_calls") => StopReason::ToolUse,
        Some("stop") | None => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        Some("insufficient_system_resource") => {
            return Err(LlmError::Retryable("insufficient_system_resource".into()))
        }
        Some(other) => StopReason::Other(other.to_string()),
    };
    let provider_state = msg
        .get("reasoning_content")
        .and_then(|r| r.as_str())
        .filter(|r| !r.is_empty())
        .map(|r| json!({"reasoning_content": r}));
    let usage = Usage {
        input: u64_at(&v, &["usage", "prompt_tokens"]),
        output: u64_at(&v, &["usage", "completion_tokens"]),
        cached: u64_at(&v, &["usage", "prompt_cache_hit_tokens"]),
        reasoning: u64_at(
            &v,
            &["usage", "completion_tokens_details", "reasoning_tokens"],
        ),
    };
    Ok(ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
            provider_state,
            provider: Some("deepseek".into()),
        },
        stop,
        usage,
        raw_id: v.get("id").and_then(|i| i.as_str()).map(String::from),
    })
}

#[async_trait::async_trait]
impl ChatProvider for DeepSeek {
    fn name(&self) -> &'static str {
        "deepseek"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let url = format!("{}/chat/completions", self.cfg.base_url);
        let headers = vec![("authorization", format!("Bearer {}", self.cfg.api_key))];
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

    #[test]
    fn reasoning_content_roundtrips_and_tool_shape() {
        let h = reqwest::header::HeaderMap::new();
        let r = parse(200, &h, r#"{"id":"c1","choices":[{"message":{"role":"assistant","content":null,"reasoning_content":"thinking…","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_x","arguments":"{\"flight\":\"a\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":80,"completion_tokens":25,"prompt_cache_hit_tokens":30,"completion_tokens_details":{"reasoning_tokens":10}}}"#).unwrap();
        assert_eq!(r.stop, StopReason::ToolUse);
        assert_eq!(
            r.usage,
            Usage {
                input: 80,
                output: 25,
                cached: 30,
                reasoning: 10
            }
        );
        let req = ChatRequest {
            model: "deepseek-v4-flash".into(),
            system: "s".into(),
            messages: vec![
                Message::user("q"),
                r.message.clone(),
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        id: "call_1".into(),
                        name: "get_x".into(),
                        content: "{\"v\":1}".into(),
                        is_error: false,
                    }],
                    provider_state: None,
                    provider: None,
                },
            ],
            tools: vec![ToolDef::new("get_x", "d", json!({"type":"object"}))],
            max_tokens: 100,
        };
        let b = build_body(&req);
        let ms = b["messages"].as_array().unwrap();
        assert_eq!(ms[0]["role"], "system");
        assert_eq!(ms[2]["reasoning_content"], "thinking…");
        assert_eq!(
            ms[2]["tool_calls"][0]["function"]["arguments"],
            "{\"flight\":\"a\"}"
        );
        assert_eq!(ms[3]["role"], "tool");
        assert_eq!(ms[3]["tool_call_id"], "call_1");
        assert_eq!(b["tools"][0]["function"]["name"], "get_x");
    }

    #[test]
    fn finish_reasons_and_402() {
        let h = reqwest::header::HeaderMap::new();
        assert!(matches!(parse(200, &h, r#"{"choices":[{"message":{"content":"x"},"finish_reason":"insufficient_system_resource"}]}"#).unwrap_err(), LlmError::Retryable(_)));
        assert!(matches!(
            parse(402, &h, r#"{"error":{"message":"Insufficient Balance"}}"#).unwrap_err(),
            LlmError::OutOfBalance
        ));
        let r = parse(200, &h, r#"{"choices":[{"message":{"content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#).unwrap();
        assert_eq!(r.stop, StopReason::EndTurn);
        assert!(r.message.provider_state.is_none());
    }
}
