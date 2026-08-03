use std::{
    net::IpAddr,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use url::{Host, Url};

use crate::{
    cache::{Cache, cache_key},
    contract::{Meta, ToolError, redact},
    http::send_with_retry,
};

const PROVIDER: &str = "tavily";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    #[serde(default = "default_topic")]
    pub topic: String,
    pub time_range: Option<String>,
    #[serde(default = "default_results")]
    pub max_results: u8,
}

fn default_topic() -> String {
    "general".to_owned()
}
fn default_results() -> u8 {
    5
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchInput {
    pub url: String,
    pub query: Option<String>,
}

#[derive(Debug)]
pub struct ToolResponse {
    pub data: Value,
    pub meta: Meta,
}

pub struct WebClient {
    client: Client,
    api_url: String,
    api_key: String,
    cache_path: Option<PathBuf>,
}

impl WebClient {
    pub fn from_env() -> Result<Self, ToolError> {
        let api_key = std::env::var("RIPPLE_TAVILY_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ToolError::auth("缺少 RIPPLE_TAVILY_API_KEY", PROVIDER))?;
        let api_url = std::env::var("RIPPLE_TAVILY_API_URL")
            .unwrap_or_else(|_| "https://api.tavily.com".to_owned());
        let cache_path = std::env::var_os("RIPPLE_TOOL_CACHE_DB").map(PathBuf::from);
        Ok(Self {
            client: Client::builder()
                .user_agent("RippleLiveTool/0.1")
                .build()
                .map_err(|error| {
                    ToolError::new("UPSTREAM_ERROR", error.to_string(), false).provider(PROVIDER)
                })?,
            api_url: api_url.trim_end_matches('/').to_owned(),
            api_key,
            cache_path,
        })
    }

    pub async fn search(&self, mut input: SearchInput) -> Result<ToolResponse, ToolError> {
        input.query = bounded_text("query", &input.query, 200)?;
        if !matches!(input.topic.as_str(), "general" | "news") {
            return Err(ToolError::invalid("topic 只能是 general 或 news"));
        }
        if let Some(range) = input.time_range.as_deref()
            && !matches!(range, "day" | "week" | "month" | "year")
        {
            return Err(ToolError::invalid(
                "time_range 只能是 day、week、month 或 year",
            ));
        }
        if !(1..=8).contains(&input.max_results) {
            return Err(ToolError::invalid("max_results 必须在 1 到 8 之间"));
        }

        let cache_input = json!({
            "query": input.query,
            "topic": input.topic,
            "time_range": input.time_range,
            "max_results": input.max_results,
        });
        let key = cache_key("web-search", &cache_input);
        if let Some(data) = self.cache_get(&key) {
            return Ok(ToolResponse {
                data,
                meta: Meta {
                    provider: PROVIDER,
                    cached: true,
                    ..Meta::default()
                },
            });
        }

        let started = Instant::now();
        let endpoint = format!("{}/search", self.api_url);
        let request = json!({
            "query": cache_input["query"],
            "topic": cache_input["topic"],
            "time_range": cache_input["time_range"],
            "max_results": cache_input["max_results"],
            "search_depth": "basic",
            "include_answer": false,
            "include_raw_content": false
        });
        let result = send_with_retry(PROVIDER, 3, Duration::from_secs(12), || {
            self.client
                .post(&endpoint)
                .bearer_auth(&self.api_key)
                .json(&request)
        })
        .await
        .map_err(|error| sanitize_error(error, &self.api_key))?;
        let payload: Value = result.response.json().await.map_err(|_| {
            ToolError::new("UPSTREAM_ERROR", "Tavily 返回了无效 JSON", true)
                .provider(PROVIDER)
                .elapsed(started)
        })?;
        let results = payload
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if results.is_empty() {
            return Err(ToolError::new("NO_RESULTS", "本次搜索没有找到结果", false)
                .provider(PROVIDER)
                .elapsed(started));
        }
        let normalized = results.into_iter().take(input.max_results as usize).map(|result| {
            json!({
                "title": truncate(result.get("title").and_then(Value::as_str).unwrap_or(""), 300),
                "url": result.get("url").and_then(Value::as_str).unwrap_or(""),
                "snippet": truncate(result.get("content").and_then(Value::as_str).unwrap_or(""), 1200),
                "published_at": result.get("published_date").cloned().unwrap_or(Value::Null),
                "score": result.get("score").cloned().unwrap_or(Value::Null)
            })
        }).collect::<Vec<_>>();
        let data =
            json!({"query": input.query, "result_count": normalized.len(), "results": normalized});
        self.cache_put(&key, 300, &data);
        Ok(ToolResponse {
            data,
            meta: Meta {
                provider: PROVIDER,
                request_id: payload
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                elapsed_ms: started.elapsed().as_millis(),
                cached: false,
                usage_credits: payload.pointer("/usage/credits").and_then(Value::as_u64),
                retry_count: result.retry_count,
            },
        })
    }

    pub async fn fetch(&self, mut input: FetchInput) -> Result<ToolResponse, ToolError> {
        validate_public_url(&input.url)?;
        input.query = input
            .query
            .map(|query| bounded_text("query", &query, 200))
            .transpose()?;
        let cache_input = json!({"url": input.url, "query": input.query});
        let key = cache_key("web-fetch", &cache_input);
        if let Some(data) = self.cache_get(&key) {
            return Ok(ToolResponse {
                data,
                meta: Meta {
                    provider: PROVIDER,
                    cached: true,
                    ..Meta::default()
                },
            });
        }

        let started = Instant::now();
        let endpoint = format!("{}/extract", self.api_url);
        let request = json!({
            "urls": [cache_input["url"].clone()],
            "query": cache_input["query"],
            "extract_depth": "basic",
            "format": "markdown"
        });
        let result = send_with_retry(PROVIDER, 2, Duration::from_secs(20), || {
            self.client
                .post(&endpoint)
                .bearer_auth(&self.api_key)
                .json(&request)
        })
        .await
        .map_err(|error| sanitize_error(error, &self.api_key))?;
        let payload: Value = result.response.json().await.map_err(|_| {
            ToolError::new("UPSTREAM_ERROR", "Tavily 返回了无效 JSON", true)
                .provider(PROVIDER)
                .elapsed(started)
        })?;
        let extracted = payload.pointer("/results/0").ok_or_else(|| {
            ToolError::new("CONTENT_BLOCKED", "网页正文无法读取", false)
                .provider(PROVIDER)
                .elapsed(started)
        })?;
        let raw = extracted
            .get("raw_content")
            .or_else(|| extracted.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if raw.is_empty() {
            return Err(ToolError::new("CONTENT_BLOCKED", "网页正文为空", false)
                .provider(PROVIDER)
                .elapsed(started));
        }
        let content = truncate(raw, 12_000);
        let data = json!({
            "url": extracted.get("url").and_then(Value::as_str).unwrap_or(&input.url),
            "content": content,
            "content_chars": raw.chars().count(),
            "truncated": raw.chars().count() > 12_000
        });
        self.cache_put(&key, 1800, &data);
        Ok(ToolResponse {
            data,
            meta: Meta {
                provider: PROVIDER,
                request_id: payload
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                elapsed_ms: started.elapsed().as_millis(),
                cached: false,
                usage_credits: payload.pointer("/usage/credits").and_then(Value::as_u64),
                retry_count: result.retry_count,
            },
        })
    }

    fn cache_get(&self, key: &str) -> Option<Value> {
        self.cache_path
            .as_deref()
            .and_then(|path| Cache::open(path).ok()?.get(key).ok().flatten())
    }

    fn cache_put(&self, key: &str, ttl: i64, data: &Value) {
        if let Some(path) = self.cache_path.as_deref()
            && let Ok(cache) = Cache::open(path)
        {
            let _ = cache.put(key, ttl, data);
        }
    }
}

fn bounded_text(name: &str, value: &str, limit: usize) -> Result<String, ToolError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ToolError::invalid(format!("{name} 不能为空")));
    }
    if value.chars().count() > limit {
        return Err(ToolError::invalid(format!(
            "{name} 不能超过 {limit} 个字符"
        )));
    }
    Ok(value.to_owned())
}

fn truncate(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn validate_public_url(value: &str) -> Result<(), ToolError> {
    let url = Url::parse(value).map_err(|_| ToolError::invalid("url 不是有效的绝对 URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::invalid("url 仅允许 http 或 https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ToolError::invalid("url 不允许包含认证信息"));
    }
    let host = url
        .host()
        .ok_or_else(|| ToolError::invalid("url 缺少主机名"))?;
    match host {
        Host::Ipv4(ip) => reject_ip(IpAddr::V4(ip))?,
        Host::Ipv6(ip) => reject_ip(IpAddr::V6(ip))?,
        Host::Domain(domain) => {
            let domain = domain.to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") || domain.ends_with(".local")
            {
                return Err(ToolError::invalid("url 不允许访问本机或内部域名"));
            }
            if let Ok(ip) = IpAddr::from_str(&domain) {
                reject_ip(ip)?;
            }
        }
    }
    Ok(())
}

fn reject_ip(ip: IpAddr) -> Result<(), ToolError> {
    let blocked = match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    };
    if blocked {
        Err(ToolError::invalid("url 不允许访问本机、私网或链路本地地址"))
    } else {
        Ok(())
    }
}

fn sanitize_error(mut error: ToolError, secret: &str) -> ToolError {
    error.message = redact(&error.message, &[secret]);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    #[test]
    fn blocks_private_urls_and_accepts_public_https() {
        assert!(validate_public_url("https://example.com/a").is_ok());
        assert!(validate_public_url("http://127.0.0.1/admin").is_err());
        assert!(validate_public_url("http://192.168.1.2/").is_err());
        assert!(validate_public_url("file:///etc/passwd").is_err());
        assert!(validate_public_url("https://user:pass@example.com").is_err());
    }

    #[test]
    fn validates_search_ranges() {
        assert!(bounded_text("query", "  ", 200).is_err());
        assert_eq!(truncate("测试内容", 2), "测试");
    }

    #[tokio::test]
    async fn normalizes_tavily_search_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "request_id": "req-1",
                "usage": {"credits": 1},
                "results": [{
                    "title": "Ripple",
                    "url": "https://example.com/ripple",
                    "content": "A useful source",
                    "published_date": "2026-08-03",
                    "score": 0.9
                }]
            })))
            .mount(&server)
            .await;
        let client = WebClient {
            client: Client::new(),
            api_url: server.uri(),
            api_key: "test-key".into(),
            cache_path: None,
        };
        let output = client
            .search(SearchInput {
                query: "Ripple".into(),
                topic: "general".into(),
                time_range: None,
                max_results: 5,
            })
            .await
            .unwrap();
        assert_eq!(output.data["result_count"], 1);
        assert_eq!(
            output.data["results"][0]["url"],
            "https://example.com/ripple"
        );
        assert_eq!(output.meta.request_id.as_deref(), Some("req-1"));
        assert_eq!(output.meta.usage_credits, Some(1));
    }

    #[tokio::test]
    async fn maps_tavily_auth_failures() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let client = WebClient {
            client: Client::new(),
            api_url: server.uri(),
            api_key: "secret-key".into(),
            cache_path: None,
        };
        let error = client
            .search(SearchInput {
                query: "Ripple".into(),
                topic: "general".into(),
                time_range: None,
                max_results: 5,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "AUTH_MISSING");
        assert!(!error.message.contains("secret-key"));
    }
}
