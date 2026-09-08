//! Retry with backoff for transient provider failures.

use crate::LlmError;
use std::future::Future;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub base: Duration,
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            base: Duration::from_secs(1),
            max_retry_after: Duration::from_secs(20),
        }
    }
}

/// Injectable sleep so tests run instantly.
#[async_trait::async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, d: Duration);
}

pub struct TokioSleeper;

#[async_trait::async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// Delay before attempt `n` (1-based) after error `e`.
pub fn delay_for(policy: &RetryPolicy, n: u32, e: &LlmError) -> Duration {
    if let LlmError::RateLimited {
        retry_after: Some(d),
    } = e
    {
        return (*d).min(policy.max_retry_after);
    }
    let mult = 1u32 << (n.saturating_sub(1)).min(4);
    let jitter_ms = (n as u64 * 137) % 250;
    policy.base * mult + Duration::from_millis(jitter_ms)
}

pub async fn with_retry<T, F, Fut>(
    policy: &RetryPolicy,
    sleeper: &dyn Sleeper,
    mut f: F,
) -> Result<T, LlmError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, LlmError>>,
{
    let mut n = 0u32;
    loop {
        n += 1;
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if e.is_retryable() && n < policy.attempts => {
                let d = delay_for(policy, n, &e);
                tracing::warn!(attempt = n, ?d, "retrying provider call: {e}");
                sleeper.sleep(d).await;
            }
            Err(e) => return Err(e),
        }
    }
}
