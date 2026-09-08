//! Provider-agnostic chat + tool-calling client and the agent loop used by the
//! AI helper. Wire formats verified against the vendors' docs on 2026-09-08
//! (see the provider modules). No key ever reaches a transcript, an event or
//! an error message ([`redact`]).

pub mod agent;
pub mod prompt;
pub mod providers;
pub mod redact;
pub mod retry;
pub mod tools;
pub mod transcript;

pub use agent::{Agent, AgentConfig, AgentEvent, AgentTurn};
pub use appconfig::{estimate_cost, ModelPrice, Provider};
pub use tools::{ToolDef, ToolHost, MAX_TOOL_RESULT_CHARS};
pub use transcript::{ToolCallRecord, Transcript, TurnMeta};

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    /// Carrier for tool results (fanned out per provider).
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
    /// Provider-opaque state echoed back verbatim on the next turn (OpenAI
    /// reasoning/function_call items, DeepSeek `reasoning_content`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<serde_json::Value>,
    /// Which provider produced `provider_state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Text { text: text.into() }],
            provider_state: None,
            provider: None,
        }
    }
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![Part::Text { text: text.into() }],
            provider_state: None,
            provider: None,
        }
    }
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn tool_uses(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolUse { id, name, args } => Some((id.as_str(), name.as_str(), args)),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub reasoning: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, o: Self) {
        self.input += o.input;
        self.output += o.output;
        self.cached += o.cached;
        self.reasoning += o.reasoning;
    }
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: Message,
    pub stop: StopReason,
    pub usage: Usage,
    pub raw_id: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("rate limited{}", retry_after.map(|d| format!(" (retry after {:.0} s)", d.as_secs_f32())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },
    #[error("temporary failure: {0}")]
    Retryable(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("provider account out of balance")]
    OutOfBalance,
    #[error("request timed out")]
    Timeout,
    #[error("could not decode the provider response: {0}")]
    Decode(String),
    #[error("token budget exhausted: {used} of {limit} tokens used this session")]
    Budget { used: u64, limit: u64 },
    #[error("tool error: {0}")]
    Tool(String),
    #[error("configuration error: {0}")]
    Config(String),
}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimited { .. } | LlmError::Retryable(_) | LlmError::Timeout
        ) || matches!(self, LlmError::Http { status, .. } if *status >= 500)
    }
    /// Scrub secrets from the message text.
    pub fn scrubbed(self, secrets: &[&str]) -> LlmError {
        match self {
            LlmError::Http { status, body } => LlmError::Http {
                status,
                body: redact::scrub(&body, secrets),
            },
            LlmError::Retryable(m) => LlmError::Retryable(redact::scrub(&m, secrets)),
            LlmError::Auth(m) => LlmError::Auth(redact::scrub(&m, secrets)),
            LlmError::Decode(m) => LlmError::Decode(redact::scrub(&m, secrets)),
            LlmError::Tool(m) => LlmError::Tool(redact::scrub(&m, secrets)),
            LlmError::Config(m) => LlmError::Config(redact::scrub(&m, secrets)),
            other => other,
        }
    }
}

#[async_trait::async_trait]
pub trait ChatProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn model(&self) -> &str;
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError>;
}

/// Connection settings for one provider.
#[derive(Clone)]
pub struct ProviderCfg {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for ProviderCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCfg")
            .field("api_key", &"****")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish()
    }
}

impl ProviderCfg {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            timeout: Duration::from_secs(90),
        }
    }
}

/// Build a provider client. Only `https://` hosts are accepted unless the
/// `PIDTUNER_LLM_ALLOW_HTTP` environment variable is set (tests, local proxies).
pub fn make_provider(
    p: Provider,
    cfg: ProviderCfg,
    client: reqwest::Client,
) -> Result<Box<dyn ChatProvider>, LlmError> {
    if !cfg.base_url.starts_with("https://")
        && std::env::var_os("PIDTUNER_LLM_ALLOW_HTTP").is_none()
    {
        return Err(LlmError::Config(format!(
            "base URL must use https:// (got {})",
            cfg.base_url
        )));
    }
    if cfg.api_key.trim().is_empty() {
        return Err(LlmError::Config(format!(
            "API key for {} is empty",
            p.title()
        )));
    }
    Ok(match p {
        Provider::Anthropic => Box::new(providers::anthropic::Anthropic { cfg, client }),
        Provider::Openai => Box::new(providers::openai::OpenAi { cfg, client }),
        Provider::Gemini => Box::new(providers::gemini::Gemini { cfg, client }),
        Provider::Deepseek => Box::new(providers::deepseek::DeepSeek { cfg, client }),
    })
}

/// Default HTTP client (rustls, 90 s).
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .expect("reqwest client")
}
