pub mod anthropic;
pub mod deepseek;
pub mod gemini;
pub mod openai;

use crate::LlmError;
use reqwest::header::HeaderMap;
use std::time::Duration;

/// POST JSON and return (status, headers, body). Network failures map to
/// `Retryable`, timeouts to `Timeout`.
pub(crate) async fn post_json(
    client: &reqwest::Client,
    url: &str,
    headers: Vec<(&'static str, String)>,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<(u16, HeaderMap, String), LlmError> {
    let mut req = client.post(url).timeout(timeout).json(body);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            LlmError::Timeout
        } else {
            LlmError::Retryable(format!("network: {e}"))
        }
    })?;
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let text = resp
        .text()
        .await
        .map_err(|e| LlmError::Retryable(format!("read body: {e}")))?;
    Ok((status, headers, text))
}

/// Shared HTTP-status → error mapping (body already extracted as a message).
pub(crate) fn status_error(status: u16, headers: &HeaderMap, message: String) -> LlmError {
    match status {
        401 | 403 => LlmError::Auth(message),
        402 => LlmError::OutOfBalance,
        429 => LlmError::RateLimited {
            retry_after: headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs),
        },
        500..=599 => LlmError::Retryable(format!("HTTP {status}: {message}")),
        _ => LlmError::Http {
            status,
            body: message,
        },
    }
}

pub(crate) fn json_or_decode(body: &str) -> Result<serde_json::Value, LlmError> {
    serde_json::from_str(body).map_err(|e| {
        LlmError::Decode(format!(
            "{e}: {}",
            body.chars().take(300).collect::<String>()
        ))
    })
}

/// Parse tool-call arguments that arrive either as an object or as a JSON string.
pub(crate) fn args_value(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => {
            serde_json::from_str(s).unwrap_or(serde_json::Value::String(s.clone()))
        }
        other => other.clone(),
    }
}

pub(crate) fn u64_at(v: &serde_json::Value, path: &[&str]) -> u64 {
    let mut cur = v;
    for p in path {
        cur = match cur.get(p) {
            Some(x) => x,
            None => return 0,
        };
    }
    cur.as_u64().unwrap_or(0)
}
