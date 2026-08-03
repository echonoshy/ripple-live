use std::time::{Duration, Instant};

use rand::Rng;
use reqwest::{RequestBuilder, Response, StatusCode};

use crate::contract::ToolError;

pub struct HttpResult {
    pub response: Response,
    pub retry_count: u32,
}

pub async fn send_with_retry<F>(
    provider: &'static str,
    attempts: u32,
    timeout: Duration,
    build: F,
) -> Result<HttpResult, ToolError>
where
    F: Fn() -> RequestBuilder,
{
    let started = Instant::now();
    for attempt in 0..attempts {
        let result = tokio::time::timeout(timeout, build().send()).await;
        match result {
            Ok(Ok(response)) if response.status().is_success() => {
                return Ok(HttpResult {
                    response,
                    retry_count: attempt,
                });
            }
            Ok(Ok(response)) => {
                let status = response.status();
                let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                if retryable && attempt + 1 < attempts {
                    backoff(attempt).await;
                    continue;
                }
                let code = match status {
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "AUTH_MISSING",
                    StatusCode::TOO_MANY_REQUESTS => "RATE_LIMITED",
                    _ => "UPSTREAM_ERROR",
                };
                return Err(ToolError::new(
                    code,
                    format!("{provider} 返回 HTTP {}", status.as_u16()),
                    retryable,
                )
                .provider(provider)
                .elapsed(started));
            }
            Ok(Err(_)) | Err(_) if attempt + 1 < attempts => {
                backoff(attempt).await;
            }
            Ok(Err(error)) => {
                return Err(ToolError::new(
                    "UPSTREAM_ERROR",
                    format!("{provider} 网络请求失败: {error}"),
                    true,
                )
                .provider(provider)
                .elapsed(started));
            }
            Err(_) => {
                return Err(ToolError::new(
                    "UPSTREAM_TIMEOUT",
                    format!("{provider} 请求超时"),
                    true,
                )
                .provider(provider)
                .elapsed(started));
            }
        }
    }
    unreachable!("attempts is always at least one")
}

async fn backoff(attempt: u32) {
    let base = 150_u64.saturating_mul(1_u64 << attempt.min(4));
    let jitter = rand::rng().random_range(0..100_u64);
    tokio::time::sleep(Duration::from_millis(base + jitter)).await;
}
