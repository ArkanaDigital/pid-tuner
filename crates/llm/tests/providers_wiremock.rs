//! One full tool round trip per provider against a mocked HTTP server, plus
//! retry / auth / timeout / redaction behaviour.
use llm::retry::{RetryPolicy, Sleeper};
use llm::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Host;
#[async_trait::async_trait]
impl ToolHost for Host {
    fn defs(&self) -> Vec<ToolDef> {
        vec![ToolDef::new(
            "get_log_quality",
            "Log quality",
            json!({"type":"object","properties":{"flight":{"type":"string","enum":["a","b","c"]}},"required":["flight"],"additionalProperties":false}),
        )]
    }
    async fn call(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        assert_eq!(name, "get_log_quality");
        assert_eq!(args["flight"], "a");
        Ok(r#"{"fs_hz":2000,"hover_seconds":31.5}"#.into())
    }
}

struct NoSleep(Arc<Mutex<Vec<Duration>>>);
#[async_trait::async_trait]
impl Sleeper for NoSleep {
    async fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

fn agent(p: Provider, base: &str, model: &str) -> Agent {
    std::env::set_var("PIDTUNER_LLM_ALLOW_HTTP", "1");
    let prov = make_provider(
        p,
        ProviderCfg::new("sk-test-KEY-abcdefgh12345678", base, model),
        http_client(),
    )
    .unwrap();
    let mut a = Agent::new(
        prov,
        Arc::new(Host),
        AgentConfig {
            model: model.into(),
            ..Default::default()
        },
    );
    a.sleeper = Box::new(NoSleep(Arc::new(Mutex::new(vec![]))));
    a
}

async fn run(a: &Agent) -> (AgentTurn, Transcript) {
    let mut t = Transcript::default();
    let turn = a
        .run(
            "sys",
            &mut t,
            "import_a",
            "Jelaskan kualitas log".into(),
            &|_| {},
        )
        .await
        .unwrap();
    (turn, t)
}

#[tokio::test]
async fn anthropic_round_trip() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(header("x-api-key", "sk-test-KEY-abcdefgh12345678")).and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({"tools":[{"name":"get_log_quality"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"msg_1","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"get_log_quality","input":{"flight":"a"}}],"stop_reason":"tool_use","usage":{"input_tokens":120,"output_tokens":30}})))
        .up_to_n_times(1).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .and(body_partial_json(json!({"messages":[{"role":"user"},{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1"}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":false}]}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"msg_2","content":[{"type":"text","text":"Hover 31.5 s, cukup."}],"stop_reason":"end_turn","usage":{"input_tokens":200,"output_tokens":12}})))
        .expect(1).mount(&s).await;
    let (turn, t) = run(&agent(Provider::Anthropic, &s.uri(), "claude-sonnet-5")).await;
    assert_eq!(turn.reply, "Hover 31.5 s, cukup.");
    assert_eq!(
        turn.usage,
        Usage {
            input: 320,
            output: 42,
            cached: 0,
            reasoning: 0
        }
    );
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(t.messages.len(), 4);
}

#[tokio::test]
async fn openai_round_trip_replays_reasoning_items() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses")).and(header("authorization", "Bearer sk-test-KEY-abcdefgh12345678"))
        .and(body_partial_json(json!({"store":false,"tools":[{"type":"function","name":"get_log_quality"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"resp_1","output":[{"type":"reasoning","id":"rs_1","summary":[]},{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_log_quality","arguments":"{\"flight\":\"a\"}"}],"usage":{"input_tokens":100,"output_tokens":20}})))
        .up_to_n_times(1).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .and(body_partial_json(json!({"input":[{"role":"user"},{"type":"reasoning","id":"rs_1"},{"type":"function_call","call_id":"call_1"},{"type":"function_call_output","call_id":"call_1"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"resp_2","output":[{"type":"message","id":"m","role":"assistant","content":[{"type":"output_text","text":"OK"}]}],"usage":{"input_tokens":150,"output_tokens":5,"input_tokens_details":{"cached_tokens":90}}})))
        .expect(1).mount(&s).await;
    let (turn, _) = run(&agent(Provider::Openai, &s.uri(), "gpt-5.6-terra")).await;
    assert_eq!(turn.reply, "OK");
    assert_eq!(turn.usage.cached, 90);
}

#[tokio::test]
async fn gemini_round_trip_matches_function_response_name() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1beta/models/gemini-3.8-flash:generateContent")).and(header("x-goog-api-key", "sk-test-KEY-abcdefgh12345678"))
        .and(body_partial_json(json!({"tools":[{"function_declarations":[{"name":"get_log_quality"}]}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_log_quality","args":{"flight":"a"}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":90,"candidatesTokenCount":15}})))
        .up_to_n_times(1).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/v1beta/models/gemini-3.8-flash:generateContent"))
        .and(body_partial_json(json!({"contents":[{"role":"user"},{"role":"model","parts":[{"functionCall":{"name":"get_log_quality"}}]},{"role":"user","parts":[{"functionResponse":{"name":"get_log_quality","response":{"hover_seconds":31.5}}}]}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Sip"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":3}})))
        .expect(1).mount(&s).await;
    let (turn, _) = run(&agent(Provider::Gemini, &s.uri(), "gemini-3.8-flash")).await;
    assert_eq!(turn.reply, "Sip");
}

#[tokio::test]
async fn deepseek_round_trip_echoes_reasoning_content() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(header("authorization", "Bearer sk-test-KEY-abcdefgh12345678"))
        .and(body_partial_json(json!({"messages":[{"role":"system"},{"role":"user"}],"tools":[{"type":"function","function":{"name":"get_log_quality"}}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"c1","choices":[{"message":{"role":"assistant","content":null,"reasoning_content":"thinking…","tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_log_quality","arguments":"{\"flight\":\"a\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":80,"completion_tokens":25,"prompt_cache_hit_tokens":0,"completion_tokens_details":{"reasoning_tokens":10}}})))
        .up_to_n_times(1).expect(1).mount(&s).await;
    Mock::given(method("POST")).and(path("/chat/completions"))
        .and(body_partial_json(json!({"messages":[{"role":"system"},{"role":"user"},{"role":"assistant","reasoning_content":"thinking…","tool_calls":[{"id":"call_1"}]},{"role":"tool","tool_call_id":"call_1"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"c2","choices":[{"message":{"role":"assistant","content":"Beres"},"finish_reason":"stop"}],"usage":{"prompt_tokens":90,"completion_tokens":2}})))
        .expect(1).mount(&s).await;
    let (turn, _) = run(&agent(Provider::Deepseek, &s.uri(), "deepseek-v4-flash")).await;
    assert_eq!(turn.reply, "Beres");
    assert_eq!(turn.usage.reasoning, 10);
}

#[tokio::test]
async fn retry_on_429_then_success_and_no_retry_on_401() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_json(json!({"error":{"message":"rate"}})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})))
        .expect(1).mount(&s).await;
    let sleeps = Arc::new(Mutex::new(vec![]));
    let mut a = agent(Provider::Anthropic, &s.uri(), "claude-sonnet-5");
    a.sleeper = Box::new(NoSleep(sleeps.clone()));
    let (turn, _) = run(&a).await;
    assert_eq!(turn.reply, "ok");
    assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(1)]);

    let s2 = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key sk-test-KEY-abcdefgh12345678"}}))).expect(1).mount(&s2).await;
    let a = agent(Provider::Anthropic, &s2.uri(), "claude-sonnet-5");
    let mut t = Transcript::default();
    let e = a
        .run("sys", &mut t, "x", "hi".into(), &|_| {})
        .await
        .unwrap_err();
    let msg = e.to_string();
    assert!(matches!(e, LlmError::Auth(_)));
    assert!(
        !msg.contains("abcdefgh12345678") && !msg.contains("12345678"),
        "{msg}"
    );
}

#[tokio::test]
async fn three_server_errors_give_up_and_402_is_out_of_balance() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error":{"message":"boom"}})))
        .expect(3)
        .mount(&s)
        .await;
    let a = agent(Provider::Deepseek, &s.uri(), "deepseek-v4-flash");
    let mut t = Transcript::default();
    let e = a
        .run("sys", &mut t, "x", "hi".into(), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(e, LlmError::Retryable(_)), "{e:?}");
    let s2 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(402)
                .set_body_json(json!({"error":{"message":"Insufficient Balance"}})),
        )
        .expect(1)
        .mount(&s2)
        .await;
    let a = agent(Provider::Deepseek, &s2.uri(), "deepseek-v4-flash");
    let e = a
        .run("sys", &mut Transcript::default(), "x", "hi".into(), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(e, LlmError::OutOfBalance));
}

#[tokio::test]
async fn timeout_maps_to_timeout_error() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(2))
                .set_body_json(json!({})),
        )
        .mount(&s)
        .await;
    std::env::set_var("PIDTUNER_LLM_ALLOW_HTTP", "1");
    let mut cfg = ProviderCfg::new("sk-test-KEY-abcdefgh12345678", s.uri(), "claude-sonnet-5");
    cfg.timeout = Duration::from_millis(200);
    let prov = make_provider(Provider::Anthropic, cfg, http_client()).unwrap();
    let mut a = Agent::new(
        prov,
        Arc::new(Host),
        AgentConfig {
            model: "claude-sonnet-5".into(),
            ..Default::default()
        },
    );
    a.retry = RetryPolicy {
        attempts: 1,
        ..Default::default()
    };
    let e = a
        .run("sys", &mut Transcript::default(), "x", "hi".into(), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(e, LlmError::Timeout), "{e:?}");
}
