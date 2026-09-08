//! Tool definitions and the host interface the agent calls into.

use serde::{Deserialize, Serialize};

pub const MAX_TOOL_RESULT_CHARS: usize = 12_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments object.
    pub schema: serde_json::Value,
}

impl ToolDef {
    pub fn new(name: &str, description: &str, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }
}

#[async_trait::async_trait]
pub trait ToolHost: Send + Sync {
    fn defs(&self) -> Vec<ToolDef>;
    /// `Ok(json_text)` or `Err(message)`; errors are reported back to the model
    /// as `is_error` tool results, they never abort the turn.
    async fn call(&self, name: &str, args: serde_json::Value) -> Result<String, String>;
}

/// Truncate a tool result to the context budget.
pub fn truncate_result(s: String) -> String {
    if s.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    out.push_str("…[dipotong]");
    out
}
