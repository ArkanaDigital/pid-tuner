//! Separate process: the allow-http env var set by the wiremock tests must not leak here.
use llm::*;

#[test]
fn https_is_required_outside_tests() {
    std::env::remove_var("PIDTUNER_LLM_ALLOW_HTTP");
    let e = make_provider(
        Provider::Openai,
        ProviderCfg::new("k-123456", "http://proxy.local", "gpt-5.6-terra"),
        http_client(),
    )
    .err()
    .unwrap();
    assert!(matches!(e, LlmError::Config(_)));
    assert!(make_provider(
        Provider::Openai,
        ProviderCfg::new("", "https://api.openai.com", "m"),
        http_client()
    )
    .is_err());
    assert!(make_provider(
        Provider::Openai,
        ProviderCfg::new("k-123456", "https://api.openai.com", "m"),
        http_client()
    )
    .is_ok());
    assert!(make_provider(
        Provider::Gemini,
        ProviderCfg::new(
            "k-123456",
            "https://generativelanguage.googleapis.com/",
            "m"
        ),
        http_client()
    )
    .is_ok());
}
