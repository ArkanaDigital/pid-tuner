//! Live round-trip against the real DeepSeek API. Runs only when
//! `DEEPSEEK_API_KEY` is set (never in CI); proves the wire format with a real
//! tool call, not just the mocked shapes.
use std::sync::{Arc, Mutex};

use appconfig::{prices, Provider};
use llm::{
    agent::{Agent, AgentConfig},
    make_provider,
    tools::{ToolDef, ToolHost},
    transcript::Transcript,
    ProviderCfg,
};

struct Host {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ToolHost for Host {
    fn defs(&self) -> Vec<ToolDef> {
        vec![ToolDef::new(
            "get_step_response",
            "Step-response metrics of one axis of the flight log.",
            serde_json::json!({"type": "object", "properties": {"axis": {"type": "string", "enum": ["roll", "pitch", "yaw"]}}, "required": ["axis"], "additionalProperties": false}),
        )]
    }
    async fn call(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        self.calls.lock().unwrap().push(format!("{name}:{args}"));
        Ok(r#"{"axis":"pitch","overshoot":1.31,"latency_ms":9.8,"n_segments":42}"#.into())
    }
}

#[tokio::test]
async fn deepseek_tool_round_trip() {
    let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else {
        eprintln!("DEEPSEEK_API_KEY not set, skipping live test");
        return;
    };
    let p = Provider::Deepseek;
    let model = prices::default_model(p).to_string();
    let provider = make_provider(
        p,
        ProviderCfg::new(key, prices::default_base_url(p), model.clone()),
        llm::http_client(),
    )
    .unwrap();
    let host = Arc::new(Host {
        calls: Mutex::new(vec![]),
    });
    let agent = Agent::new(
        provider,
        host.clone(),
        AgentConfig {
            model: model.clone(),
            max_rounds: 3,
            budget_tokens: 50_000,
            max_tokens: 400,
            price: prices::defaults().get(&model).cloned(),
            context_messages: 20,
        },
    );
    let mut t = Transcript::default();
    let turn = agent
        .run(
            "You are a PID tuning assistant. Always fetch data with the tools before answering. Answer in one short sentence and quote the overshoot number.",
            &mut t,
            "pid_analysis",
            "Berapa overshoot pitch di log ini?".into(),
            &|_| {},
        )
        .await
        .expect("live turn");
    let calls = host.calls.lock().unwrap().clone();
    eprintln!(
        "reply: {}\ncalls: {calls:?}\nusage: {:?} cost {:.5}",
        turn.reply, turn.usage, turn.cost_usd
    );
    assert!(
        calls.iter().any(|c| c.starts_with("get_step_response:")),
        "model did not call the tool: {calls:?}"
    );
    assert!(
        turn.reply.contains("1.31") || turn.reply.contains("1,31"),
        "reply must quote the number: {}",
        turn.reply
    );
    assert!(turn.usage.input > 0 && turn.usage.output > 0);
    assert!(turn.cost_usd > 0.0);
    assert_eq!(t.turns.len(), 1);
}
