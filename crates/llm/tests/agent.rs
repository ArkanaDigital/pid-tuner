//! Agent loop behaviour with a scripted provider and a recording tool host.
use llm::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

struct Scripted {
    responses: Mutex<std::collections::VecDeque<ChatResponse>>,
    calls: Arc<Mutex<Vec<ChatRequest>>>,
}

impl Scripted {
    fn new(r: Vec<ChatResponse>) -> Box<Self> {
        Box::new(Self {
            responses: Mutex::new(r.into()),
            calls: Arc::new(Mutex::new(vec![])),
        })
    }
    fn with_log(r: Vec<ChatResponse>, calls: Arc<Mutex<Vec<ChatRequest>>>) -> Box<Self> {
        Box::new(Self {
            responses: Mutex::new(r.into()),
            calls,
        })
    }
}

#[async_trait::async_trait]
impl ChatProvider for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }
    fn model(&self) -> &str {
        "m"
    }
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        self.calls.lock().unwrap().push(req.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| LlmError::Retryable("script exhausted".into()))
    }
}

fn text(t: &str, input: u64, output: u64) -> ChatResponse {
    ChatResponse {
        message: Message::assistant_text(t),
        stop: StopReason::EndTurn,
        usage: Usage {
            input,
            output,
            cached: 0,
            reasoning: 0,
        },
        raw_id: None,
    }
}
fn tool_use(calls: &[(&str, &str, serde_json::Value)]) -> ChatResponse {
    ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts: calls
                .iter()
                .map(|(id, n, a)| Part::ToolUse {
                    id: id.to_string(),
                    name: n.to_string(),
                    args: a.clone(),
                })
                .collect(),
            provider_state: None,
            provider: Some("scripted".into()),
        },
        stop: StopReason::ToolUse,
        usage: Usage {
            input: 10,
            output: 5,
            cached: 0,
            reasoning: 0,
        },
        raw_id: None,
    }
}

#[derive(Default)]
struct RecHost {
    calls: Mutex<Vec<(String, serde_json::Value)>>,
    fail: bool,
    big: bool,
}
#[async_trait::async_trait]
impl ToolHost for RecHost {
    fn defs(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::new("get_a", "a", json!({"type":"object"})),
            ToolDef::new("get_b", "b", json!({"type":"object"})),
        ]
    }
    async fn call(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        self.calls.lock().unwrap().push((name.into(), args));
        if self.fail {
            return Err("host exploded".into());
        }
        if self.big {
            return Ok("x".repeat(MAX_TOOL_RESULT_CHARS + 500));
        }
        Ok(format!("{{\"tool\":\"{name}\"}}"))
    }
}

fn agent(p: Box<Scripted>, host: Arc<RecHost>, cfg: AgentConfig) -> Agent {
    Agent::new(p, host, cfg)
}

#[tokio::test]
async fn text_only_single_round() {
    let host = Arc::new(RecHost::default());
    let a = agent(
        Scripted::new(vec![text("halo", 50, 7)]),
        host.clone(),
        AgentConfig::default(),
    );
    let mut t = Transcript::default();
    let turn = a
        .run("s", &mut t, "import_a", "hi".into(), &|_| {})
        .await
        .unwrap();
    assert_eq!(turn.reply, "halo");
    assert_eq!(
        turn.usage,
        Usage {
            input: 50,
            output: 7,
            cached: 0,
            reasoning: 0
        }
    );
    assert!(host.calls.lock().unwrap().is_empty());
    assert_eq!(t.messages.len(), 2);
    assert_eq!(t.turns.len(), 1);
    assert!(turn.stopped_by.is_none());
}

#[tokio::test]
async fn parallel_tool_calls_land_in_one_tool_message_in_order() {
    let host = Arc::new(RecHost::default());
    let p = Scripted::new(vec![
        tool_use(&[("1", "get_a", json!({})), ("2", "get_b", json!({"k":1}))]),
        text("done", 30, 4),
    ]);
    let a = agent(p, host.clone(), AgentConfig::default());
    let events = Arc::new(Mutex::new(vec![]));
    let ev2 = events.clone();
    let mut t = Transcript::default();
    let turn = a
        .run("s", &mut t, "x", "go".into(), &move |e| {
            ev2.lock().unwrap().push(e)
        })
        .await
        .unwrap();
    assert_eq!(turn.reply, "done");
    assert_eq!(turn.usage.total(), 49);
    let calls = host.calls.lock().unwrap();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        vec!["get_a", "get_b"]
    );
    let tool_msg = t.messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(tool_msg.parts.len(), 2);
    assert!(
        matches!(&tool_msg.parts[1], Part::ToolResult { id, is_error: false, .. } if id == "2")
    );
    let kinds: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| {
            serde_json::to_value(e).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "thinking",
            "usage",
            "tool_call",
            "tool_result",
            "tool_call",
            "tool_result",
            "thinking",
            "usage"
        ]
    );
    assert_eq!(t.usage_total.total(), 49);
}

#[tokio::test]
async fn malformed_args_and_unknown_tools_never_reach_the_host() {
    let host = Arc::new(RecHost::default());
    let p = Scripted::new(vec![
        tool_use(&[("1", "get_a", json!("notjson")), ("2", "nope", json!({}))]),
        text("ok", 1, 1),
    ]);
    let a = agent(p, host.clone(), AgentConfig::default());
    let mut t = Transcript::default();
    a.run("s", &mut t, "x", "go".into(), &|_| {}).await.unwrap();
    assert!(host.calls.lock().unwrap().is_empty());
    let tool_msg = t.messages.iter().find(|m| m.role == Role::Tool).unwrap();
    for p in &tool_msg.parts {
        assert!(matches!(p, Part::ToolResult { is_error: true, .. }));
    }
    assert!(
        matches!(&tool_msg.parts[0], Part::ToolResult { content, .. } if content.contains("expected a JSON object"))
    );
    assert!(
        matches!(&tool_msg.parts[1], Part::ToolResult { content, .. } if content.contains("unknown tool"))
    );
}

#[tokio::test]
async fn host_error_is_reported_and_loop_continues() {
    let host = Arc::new(RecHost {
        fail: true,
        ..Default::default()
    });
    let p = Scripted::new(vec![
        tool_use(&[("1", "get_a", json!({}))]),
        text("recovered", 1, 1),
    ]);
    let a = agent(p, host.clone(), AgentConfig::default());
    let mut t = Transcript::default();
    let turn = a.run("s", &mut t, "x", "go".into(), &|_| {}).await.unwrap();
    assert_eq!(turn.reply, "recovered");
    assert!(!turn.tool_calls[0].ok);
    assert!(turn.tool_calls[0].preview.contains("host exploded"));
}

#[tokio::test]
async fn max_rounds_forces_a_final_text_answer_without_tools() {
    let host = Arc::new(RecHost::default());
    // the model keeps asking for tools; after max_rounds the agent asks once more WITHOUT tools
    let mut script: Vec<ChatResponse> = (0..3)
        .map(|i| tool_use(&[(&format!("{i}"), "get_a", json!({}))]))
        .collect();
    script.push(text("final", 2, 2));
    let log = Arc::new(Mutex::new(vec![]));
    let p = Scripted::with_log(script, log.clone());
    let a = agent(
        p,
        host.clone(),
        AgentConfig {
            max_rounds: 3,
            ..Default::default()
        },
    );
    let mut t = Transcript::default();
    let turn = a.run("s", &mut t, "x", "go".into(), &|_| {}).await.unwrap();
    assert_eq!(turn.stopped_by.as_deref(), Some("max_rounds"));
    assert_eq!(host.calls.lock().unwrap().len(), 3);
    // 3 tool rounds + 1 final call; the final request carries no tools
    let calls = log.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls[3].tools.is_empty() && !calls[0].tools.is_empty());
    assert_eq!(turn.reply, "final");
}

#[tokio::test]
async fn budget_stops_before_call_and_after_a_round() {
    let host = Arc::new(RecHost::default());
    let p = Scripted::new(vec![text("x", 1, 1)]);
    let a = agent(
        p,
        host.clone(),
        AgentConfig {
            budget_tokens: 100,
            ..Default::default()
        },
    );
    let mut t = Transcript::default();
    t.usage_total = Usage {
        input: 100,
        output: 0,
        cached: 0,
        reasoning: 0,
    };
    let e = a
        .run("s", &mut t, "x", "go".into(), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(
        e,
        LlmError::Budget {
            used: 100,
            limit: 100
        }
    ));
    assert!(t.turns.is_empty());
    // budget crossed mid-loop: stop after the current round without another provider call
    let p = Scripted::new(vec![
        tool_use(&[("1", "get_a", json!({}))]),
        text("never", 1, 1),
    ]);
    let a = agent(
        p,
        host.clone(),
        AgentConfig {
            budget_tokens: 12,
            ..Default::default()
        },
    );
    let mut t = Transcript::default();
    let turn = a.run("s", &mut t, "x", "go".into(), &|_| {}).await.unwrap();
    assert_eq!(turn.stopped_by.as_deref(), Some("budget"));
    assert!(turn.reply.contains("Batas token"));
    assert_eq!(t.usage_total.total(), 15);
}

#[tokio::test]
async fn tool_results_are_truncated_and_cost_is_estimated() {
    let host = Arc::new(RecHost {
        big: true,
        ..Default::default()
    });
    let p = Scripted::new(vec![
        tool_use(&[("1", "get_a", json!({}))]),
        text("ok", 1000, 500),
    ]);
    let price = ModelPrice {
        input_per_m: 3.0,
        output_per_m: 15.0,
        cached_input_per_m: Some(0.3),
    };
    let a = agent(
        p,
        host.clone(),
        AgentConfig {
            price: Some(price),
            ..Default::default()
        },
    );
    let mut t = Transcript::default();
    let turn = a.run("s", &mut t, "x", "go".into(), &|_| {}).await.unwrap();
    let tool_msg = t.messages.iter().find(|m| m.role == Role::Tool).unwrap();
    match &tool_msg.parts[0] {
        Part::ToolResult { content, .. } => {
            assert!(content.ends_with("…[dipotong]"));
            assert!(content.chars().count() <= MAX_TOOL_RESULT_CHARS + 20);
        }
        _ => panic!(),
    }
    // 1010 in × 3 + 505 out × 15 = 3030 + 7575 = 10605 per 1e6
    assert!((turn.cost_usd - 0.010605).abs() < 1e-9, "{}", turn.cost_usd);
    assert!((t.cost_total_usd - turn.cost_usd).abs() < 1e-12);
}
