use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub provider: Option<&'static str>,
    pub elapsed_ms: Option<u128>,
    pub details: Option<Box<Value>>,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            provider: None,
            elapsed_ms: None,
            details: None,
        }
    }

    pub fn provider(mut self, provider: &'static str) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn elapsed(mut self, started: Instant) -> Self {
        self.elapsed_ms = Some(started.elapsed().as_millis());
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message, false)
    }

    pub fn auth(message: impl Into<String>, provider: &'static str) -> Self {
        Self::new("AUTH_MISSING", message, false).provider(provider)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Meta {
    pub provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub elapsed_ms: u128,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_credits: Option<u64>,
    #[serde(skip_serializing_if = "is_zero")]
    pub retry_count: u32,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            provider: "unknown",
            request_id: None,
            elapsed_ms: 0,
            cached: false,
            usage_credits: None,
            retry_count: 0,
        }
    }
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

pub fn success(data: Value, meta: Meta) -> Value {
    json!({"ok": true, "data": data, "meta": meta})
}

pub fn failure(error: ToolError) -> Value {
    let mut meta = serde_json::Map::new();
    if let Some(provider) = error.provider {
        meta.insert("provider".into(), json!(provider));
    }
    if let Some(elapsed_ms) = error.elapsed_ms {
        meta.insert("elapsed_ms".into(), json!(elapsed_ms));
    }
    let mut error_json = json!({
        "code": error.code,
        "message": error.message,
        "retryable": error.retryable
    });
    if let Some(details) = error.details {
        error_json["details"] = *details;
    }
    json!({
        "ok": false,
        "error": error_json,
        "meta": meta
    })
}

pub fn redact(input: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(input.to_owned(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_success_and_error_envelopes() {
        let ok = success(
            json!({"result_count": 1}),
            Meta {
                provider: "tavily",
                elapsed_ms: 4,
                cached: false,
                ..Meta::default()
            },
        );
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["meta"]["provider"], "tavily");

        let error = failure(ToolError::new("NO_RESULTS", "没有结果", false));
        assert_eq!(error["ok"], false);
        assert_eq!(error["error"]["code"], "NO_RESULTS");
    }

    #[test]
    fn redacts_every_known_secret() {
        assert_eq!(
            redact("key=abc jwt=def", &["abc", "def"]),
            "key=[REDACTED] jwt=[REDACTED]"
        );
    }
}
