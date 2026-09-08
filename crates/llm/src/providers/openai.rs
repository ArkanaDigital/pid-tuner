//! OpenAI Responses API (`POST /v1/responses`). Tools are flat
//! `{type:"function", name, description, parameters}`; tool calls arrive as
//! `function_call` output items with a `call_id` and a JSON *string* of
//! arguments; reasoning items must be replayed verbatim before the
//! `function_call_output` items of the next request.

use super::{args_value, json_or_decode, post_json, status_error, u64_at};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, LlmError, Message, Part, ProviderCfg, Role,
    StopReason, Usage,
};
use serde_json::{json, Value};

pub struct OpenAi {
    pub cfg: ProviderCfg,
    pub client: reqwest::Client,
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::System => continue,
            Role::User => input.push(json!({"role": "user", "content": m.text()})),
            Role::Assistant => {
                // Replay the provider's own output items when we have them (reasoning + calls).
                if let Some(Value::Array(items)) = &m.provider_state {
                    input.extend(items.iter().cloned());
                } else {
                    let text = m.text();
                    if !text.is_empty() {
                        input.push(json!({"role": "assistant", "content": [{"type": "output_text", "text": text}]}));
                    }
                    for (id, name, args) in m.tool_uses() {
                        input.push(json!({"type": "function_call", "call_id": id, "name": name, "arguments": args.to_string()}));
                    }
                }
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
                        let output = if *is_error {
                            format!("{{\"error\":{}}}", Value::String(content.clone()))
                        } else {
                            content.clone()
                        };
                        input.push(json!({"type": "function_call_output", "call_id": id, "output": output}));
                    }
                }
            }
        }
    }
    let mut body = json!({"model": req.model, "instructions": req.system, "input": input, "store": false, "max_output_tokens": req.max_tokens});
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.schema, "strict": false})).collect());
        body["parallel_tool_calls"] = json!(true);
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
    let items = v
        .get("output")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default();
    let mut parts = Vec::new();
    let mut has_call = false;
    for it in &items {
        match it.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                for c in it
                    .get("content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default()
                {
                    if c.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        parts.push(Part::Text {
                            text: c
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                }
            }
            Some("function_call") => {
                has_call = true;
                parts.push(Part::ToolUse {
                    id: it
                        .get("call_id")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    name: it
                        .get("name")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    args: args_value(it.get("arguments").unwrap_or(&Value::Null)),
                });
            }
            _ => {}
        }
    }
    let stop = if has_call {
        StopReason::ToolUse
    } else if v
        .pointer("/incomplete_details/reason")
        .and_then(|r| r.as_str())
        == Some("max_output_tokens")
    {
        StopReason::MaxTokens
    } else {
        StopReason::EndTurn
    };
    let usage = Usage {
        input: u64_at(&v, &["usage", "input_tokens"]),
        output: u64_at(&v, &["usage", "output_tokens"]),
        cached: u64_at(&v, &["usage", "input_tokens_details", "cached_tokens"]),
        reasoning: u64_at(&v, &["usage", "output_tokens_details", "reasoning_tokens"]),
    };
    Ok(ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
            provider_state: Some(Value::Array(items)),
            provider: Some("openai".into()),
        },
        stop,
        usage,
        raw_id: v.get("id").and_then(|i| i.as_str()).map(String::from),
    })
}

#[async_trait::async_trait]
impl ChatProvider for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let url = format!("{}/v1/responses", self.cfg.base_url);
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
    fn parses_function_call_with_string_arguments_and_keeps_items_for_replay() {
        let h = reqwest::header::HeaderMap::new();
        let body = r#"{"id":"resp_1","output":[{"type":"reasoning","id":"rs_1","summary":[]},{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_x","arguments":"{\"flight\":\"a\"}"}],"usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":40},"output_tokens_details":{"reasoning_tokens":7}}}"#;
        let r = parse(200, &h, body).unwrap();
        assert_eq!(r.stop, StopReason::ToolUse);
        let (id, name, args) = r.message.tool_uses()[0];
        assert_eq!((id, name), ("call_1", "get_x"));
        assert_eq!(args["flight"], "a");
        assert_eq!(
            r.usage,
            Usage {
                input: 100,
                output: 20,
                cached: 40,
                reasoning: 7
            }
        );
        // replay: reasoning + function_call items come first, then the output
        let req = ChatRequest {
            model: "gpt-5.6-terra".into(),
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
        let input = b["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(b["store"], false);
        assert_eq!(b["tools"][0]["name"], "get_x");
        assert!(b["tools"][0].get("function").is_none());
        assert_eq!(b["instructions"], "s");
    }

    #[test]
    fn parses_text_and_lenient_usage_and_max_tokens() {
        let h = reqwest::header::HeaderMap::new();
        let r = parse(200, &h, r#"{"id":"resp_2","output":[{"type":"message","id":"msg_1","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":15,"output_tokens":45}}"#).unwrap();
        assert_eq!(r.message.text(), "hello");
        assert_eq!(r.stop, StopReason::EndTurn);
        assert_eq!(r.usage.cached, 0);
        let r = parse(
            200,
            &h,
            r#"{"output":[],"incomplete_details":{"reason":"max_output_tokens"},"usage":{}}"#,
        )
        .unwrap();
        assert_eq!(r.stop, StopReason::MaxTokens);
        let e = parse(
            429,
            &h,
            r#"{"error":{"code":"rate_limit_error","type":"rate_limit","message":"slow down"}}"#,
        )
        .unwrap_err();
        assert!(matches!(e, LlmError::RateLimited { .. }));
        // assistant message without provider state is synthesised
        let m = Message {
            role: Role::Assistant,
            parts: vec![
                Part::Text { text: "t".into() },
                Part::ToolUse {
                    id: "c9".into(),
                    name: "f".into(),
                    args: json!({"a":1}),
                },
            ],
            provider_state: None,
            provider: None,
        };
        let b = build_body(&ChatRequest {
            model: "m".into(),
            system: "".into(),
            messages: vec![m],
            tools: vec![],
            max_tokens: 10,
        });
        assert_eq!(b["input"][0]["content"][0]["type"], "output_text");
        assert_eq!(b["input"][1]["arguments"], "{\"a\":1}");
    }
}
