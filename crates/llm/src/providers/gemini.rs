//! Google Gemini `generateContent` (stateless). `system_instruction`,
//! `contents[{role: user|model, parts[text|functionCall|functionResponse]}]`,
//! `tools[{function_declarations}]`. Function calls carry no id, so one is
//! synthesised (`name#index`) and results are matched back by `name`.

use super::{json_or_decode, post_json, status_error, u64_at};
use crate::{
    ChatProvider, ChatRequest, ChatResponse, LlmError, Message, Part, ProviderCfg, Role,
    StopReason, Usage,
};
use serde_json::{json, Value};

pub struct Gemini {
    pub cfg: ProviderCfg,
    pub client: reqwest::Client,
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::System => continue,
            Role::User => contents.push(json!({"role": "user", "parts": [{"text": m.text()}]})),
            Role::Assistant => {
                let mut parts: Vec<Value> = Vec::new();
                for p in &m.parts {
                    match p {
                        Part::Text { text } if !text.is_empty() => {
                            parts.push(json!({"text": text}))
                        }
                        Part::ToolUse { name, args, .. } => {
                            parts.push(json!({"functionCall": {"name": name, "args": args}}))
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role": "model", "parts": parts}));
                }
            }
            Role::Tool => {
                let parts: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolResult {
                            name,
                            content,
                            is_error,
                            ..
                        } => {
                            let response = if *is_error {
                                json!({"error": content})
                            } else {
                                serde_json::from_str::<Value>(content)
                                    .map(|v| {
                                        if v.is_object() {
                                            v
                                        } else {
                                            json!({"result": v})
                                        }
                                    })
                                    .unwrap_or(json!({"result": content}))
                            };
                            Some(json!({"functionResponse": {"name": name, "response": response}}))
                        }
                        _ => None,
                    })
                    .collect();
                contents.push(json!({"role": "user", "parts": parts}));
            }
        }
    }
    let mut body = json!({"system_instruction": {"parts": [{"text": req.system}]}, "contents": contents, "generationConfig": {"maxOutputTokens": req.max_tokens}});
    if !req.tools.is_empty() {
        body["tools"] = json!([{"function_declarations": req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "parameters": t.schema})).collect::<Vec<_>>()}]);
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
    let cand = v.pointer("/candidates/0").cloned().unwrap_or(Value::Null);
    let mut parts = Vec::new();
    let mut n_calls = 0usize;
    for p in cand
        .pointer("/content/parts")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default()
    {
        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
            parts.push(Part::Text {
                text: t.to_string(),
            });
        } else if let Some(fc) = p.get("functionCall") {
            let name = fc
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string();
            parts.push(Part::ToolUse {
                id: format!("{name}#{n_calls}"),
                name,
                args: fc.get("args").cloned().unwrap_or(json!({})),
            });
            n_calls += 1;
        }
    }
    let stop = if n_calls > 0 {
        StopReason::ToolUse
    } else {
        match cand.get("finishReason").and_then(|f| f.as_str()) {
            Some("STOP") | None => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some(other) => StopReason::Other(other.to_string()),
        }
    };
    let usage = Usage {
        input: u64_at(&v, &["usageMetadata", "promptTokenCount"]),
        output: u64_at(&v, &["usageMetadata", "candidatesTokenCount"])
            + u64_at(&v, &["usageMetadata", "thoughtsTokenCount"]),
        cached: u64_at(&v, &["usageMetadata", "cachedContentTokenCount"]),
        reasoning: u64_at(&v, &["usageMetadata", "thoughtsTokenCount"]),
    };
    Ok(ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
            provider_state: None,
            provider: Some("gemini".into()),
        },
        stop,
        usage,
        raw_id: v
            .get("responseId")
            .and_then(|i| i.as_str())
            .map(String::from),
    })
}

#[async_trait::async_trait]
impl ChatProvider for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }
    fn model(&self) -> &str {
        &self.cfg.model
    }
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            self.cfg.base_url, req.model
        );
        let headers = vec![("x-goog-api-key", self.cfg.api_key.clone())];
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
    fn function_call_gets_synthesised_id_and_response_matches_name() {
        let h = reqwest::header::HeaderMap::new();
        let r = parse(200, &h, r#"{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_x","args":{"flight":"a"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":90,"candidatesTokenCount":15,"thoughtsTokenCount":5,"cachedContentTokenCount":10}}"#).unwrap();
        assert_eq!(r.stop, StopReason::ToolUse);
        let (id, name, args) = r.message.tool_uses()[0];
        assert_eq!((id, name), ("get_x#0", "get_x"));
        assert_eq!(args["flight"], "a");
        assert_eq!(
            r.usage,
            Usage {
                input: 90,
                output: 20,
                cached: 10,
                reasoning: 5
            }
        );
        let req = ChatRequest {
            model: "gemini-3.8-flash".into(),
            system: "s".into(),
            messages: vec![
                Message::user("q"),
                r.message.clone(),
                Message {
                    role: Role::Tool,
                    parts: vec![Part::ToolResult {
                        id: "get_x#0".into(),
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
        assert_eq!(b["system_instruction"]["parts"][0]["text"], "s");
        assert_eq!(b["contents"][1]["role"], "model");
        assert_eq!(
            b["contents"][1]["parts"][0]["functionCall"]["name"],
            "get_x"
        );
        assert_eq!(b["contents"][2]["role"], "user");
        assert_eq!(
            b["contents"][2]["parts"][0]["functionResponse"]["name"],
            "get_x"
        );
        assert_eq!(
            b["contents"][2]["parts"][0]["functionResponse"]["response"]["v"],
            1
        );
        assert_eq!(b["tools"][0]["function_declarations"][0]["name"], "get_x");
    }

    #[test]
    fn safety_and_errors() {
        let h = reqwest::header::HeaderMap::new();
        let r = parse(200, &h, r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"SAFETY"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1}}"#).unwrap();
        assert_eq!(r.stop, StopReason::Other("SAFETY".into()));
        assert_eq!(r.message.text(), "hi");
        let e = parse(
            429,
            &h,
            r#"{"error":{"code":429,"message":"quota","status":"RESOURCE_EXHAUSTED"}}"#,
        )
        .unwrap_err();
        assert!(matches!(e, LlmError::RateLimited { .. }));
        let e = parse(
            403,
            &h,
            r#"{"error":{"code":403,"message":"bad key","status":"PERMISSION_DENIED"}}"#,
        )
        .unwrap_err();
        assert!(matches!(e, LlmError::Auth(_)));
    }
}
