use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey, pkcs8::DecodePrivateKey};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    cache::{Cache, cache_key},
    contract::{Meta, ToolError, redact},
    http::send_with_retry,
};

const PROVIDER: &str = "qweather";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherInput {
    pub location: String,
    #[serde(default = "default_days")]
    pub days: u8,
    #[serde(default)]
    pub include_hourly: bool,
}

fn default_days() -> u8 {
    3
}

#[derive(Debug)]
pub struct ToolResponse {
    pub data: Value,
    pub meta: Meta,
}

struct CachedResponse {
    value: Value,
    cached: bool,
    request_id: Option<String>,
    retry_count: u32,
}

pub struct WeatherClient {
    client: Client,
    api_host: String,
    project_id: String,
    credential_id: String,
    signing_key: SigningKey,
    private_key_text: String,
    cache_path: Option<PathBuf>,
}

impl WeatherClient {
    pub fn from_env() -> Result<Self, ToolError> {
        let api_host = required_env("RIPPLE_QWEATHER_API_HOST")?;
        let project_id = required_env("RIPPLE_QWEATHER_PROJECT_ID")?;
        let credential_id = required_env("RIPPLE_QWEATHER_CREDENTIAL_ID")?;
        let private_key_path = required_env("RIPPLE_QWEATHER_PRIVATE_KEY_PATH")?;
        let private_key_text = std::fs::read_to_string(&private_key_path)
            .map_err(|_| ToolError::auth("无法读取和风天气 Ed25519 私钥", PROVIDER))?;
        let signing_key = SigningKey::from_pkcs8_pem(&private_key_text)
            .map_err(|_| ToolError::auth("和风天气 Ed25519 私钥格式无效", PROVIDER))?;
        let api_host = if api_host.starts_with("http://") || api_host.starts_with("https://") {
            api_host
        } else {
            format!("https://{api_host}")
        };
        Ok(Self {
            client: Client::builder()
                .user_agent("RippleLiveTool/0.1")
                .build()
                .map_err(|error| {
                    ToolError::new("UPSTREAM_ERROR", error.to_string(), false).provider(PROVIDER)
                })?,
            api_host: api_host.trim_end_matches('/').to_owned(),
            project_id,
            credential_id,
            signing_key,
            private_key_text,
            cache_path: std::env::var_os("RIPPLE_TOOL_CACHE_DB").map(PathBuf::from),
        })
    }

    pub async fn lookup(&self, mut input: WeatherInput) -> Result<ToolResponse, ToolError> {
        input.location = input.location.trim().to_owned();
        if input.location.is_empty() {
            return Err(ToolError::invalid("location 不能为空"));
        }
        if input.location.chars().count() > 100 {
            return Err(ToolError::invalid("location 不能超过 100 个字符"));
        }
        if !(1..=7).contains(&input.days) {
            return Err(ToolError::invalid("days 必须在 1 到 7 之间"));
        }

        let started = Instant::now();
        let location = self.resolve_location(&input.location).await?;
        let location_id = location.get("id").and_then(Value::as_str).ok_or_else(|| {
            ToolError::new("UPSTREAM_ERROR", "地点结果缺少 Location ID", false).provider(PROVIDER)
        })?;
        let daily_endpoint = if input.days <= 3 { "3d" } else { "7d" };
        let now = self
            .get_cached_json(
                "weather-now",
                &format!("/v7/weather/now?location={location_id}&lang=zh"),
                600,
            )
            .await?;
        let daily = self
            .get_cached_json(
                &format!("weather-{daily_endpoint}"),
                &format!("/v7/weather/{daily_endpoint}?location={location_id}&lang=zh"),
                1800,
            )
            .await?;
        let hourly = if input.include_hourly {
            Some(
                self.get_cached_json(
                    "weather-24h",
                    &format!("/v7/weather/24h?location={location_id}&lang=zh"),
                    1800,
                )
                .await?,
            )
        } else {
            None
        };

        ensure_qweather_ok(&now.value)?;
        ensure_qweather_ok(&daily.value)?;
        if let Some(hourly) = &hourly {
            ensure_qweather_ok(&hourly.value)?;
        }

        let daily_items = daily
            .value
            .get("daily")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(input.days as usize)
            .collect::<Vec<_>>();
        let hourly_items = hourly
            .as_ref()
            .and_then(|item| item.value.get("hourly"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(24)
            .collect::<Vec<_>>();
        let data = json!({
            "location": {
                "name": location.get("name").cloned().unwrap_or(Value::Null),
                "adm1": location.get("adm1").cloned().unwrap_or(Value::Null),
                "adm2": location.get("adm2").cloned().unwrap_or(Value::Null),
                "location_id": location.get("id").cloned().unwrap_or(Value::Null),
                "latitude": location.get("lat").cloned().unwrap_or(Value::Null),
                "longitude": location.get("lon").cloned().unwrap_or(Value::Null),
                "timezone": location.get("tz").cloned().unwrap_or(Value::Null)
            },
            "updated_at": now.value.get("updateTime").cloned().unwrap_or(Value::Null),
            "now": now.value.get("now").cloned().unwrap_or(Value::Null),
            "daily": daily_items,
            "hourly": hourly_items,
            "attribution": now.value.get("refer").cloned().unwrap_or(Value::Null)
        });
        let all_cached =
            now.cached && daily.cached && hourly.as_ref().is_none_or(|item| item.cached);
        let retry_count = now.retry_count
            + daily.retry_count
            + hourly.as_ref().map_or(0, |item| item.retry_count);
        let request_id = hourly
            .as_ref()
            .and_then(|item| item.request_id.clone())
            .or(daily.request_id)
            .or(now.request_id);
        Ok(ToolResponse {
            data,
            meta: Meta {
                provider: PROVIDER,
                request_id,
                elapsed_ms: started.elapsed().as_millis(),
                cached: all_cached,
                retry_count,
                ..Meta::default()
            },
        })
    }

    async fn resolve_location(&self, query: &str) -> Result<Value, ToolError> {
        let response = self
            .get_cached_json(
                "qweather-location",
                &format!(
                    "/geo/v2/city/lookup?location={}&lang=zh&number=5",
                    urlencoding(query)
                ),
                86_400,
            )
            .await?;
        ensure_qweather_ok(&response.value)?;
        let locations = response
            .value
            .get("location")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let first = locations.first().cloned().ok_or_else(|| {
            ToolError::new("NO_RESULTS", "没有找到这个地点", false).provider(PROVIDER)
        })?;
        let first_name = first.get("name").and_then(Value::as_str).unwrap_or("");
        let ambiguous = locations.iter().skip(1).any(|item| {
            item.get("name").and_then(Value::as_str) == Some(first_name)
                && item.get("id") != first.get("id")
        });
        if ambiguous {
            let candidates = locations.into_iter().take(5).map(|item| json!({
                "name": item.get("name"), "adm1": item.get("adm1"), "adm2": item.get("adm2"), "location_id": item.get("id")
            })).collect::<Vec<_>>();
            return Err(ToolError::new(
                "AMBIGUOUS_LOCATION",
                "地点名称有多个匹配，请补充省市信息",
                false,
            )
            .provider(PROVIDER)
            .details(json!({"candidates": candidates})));
        }
        Ok(first)
    }

    async fn get_cached_json(
        &self,
        namespace: &str,
        path_and_query: &str,
        ttl: i64,
    ) -> Result<CachedResponse, ToolError> {
        let key = cache_key(namespace, &json!({"path": path_and_query}));
        if let Some(value) = self.cache_get(&key) {
            return Ok(CachedResponse {
                value,
                cached: true,
                request_id: None,
                retry_count: 0,
            });
        }
        let token = self.jwt()?;
        let endpoint = format!("{}{}", self.api_host, path_and_query);
        let started = Instant::now();
        let result = send_with_retry(PROVIDER, 3, Duration::from_secs(10), || {
            self.client.get(&endpoint).bearer_auth(&token)
        })
        .await
        .map_err(|mut error| {
            error.message = redact(&error.message, &[&token, &self.private_key_text]);
            error
        })?;
        let request_id = result
            .response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let value: Value = result.response.json().await.map_err(|_| {
            ToolError::new("UPSTREAM_ERROR", "和风天气返回了无效 JSON", true)
                .provider(PROVIDER)
                .elapsed(started)
        })?;
        self.cache_put(&key, ttl, &value);
        Ok(CachedResponse {
            value,
            cached: false,
            request_id,
            retry_count: result.retry_count,
        })
    }

    fn jwt(&self) -> Result<String, ToolError> {
        let now = chrono::Utc::now().timestamp();
        let header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"alg": "EdDSA", "kid": self.credential_id}))
                .map_err(|_| ToolError::auth("无法生成和风天气 JWT", PROVIDER))?,
        );
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"sub": self.project_id, "iat": now - 30, "exp": now + 900}))
                .map_err(|_| ToolError::auth("无法生成和风天气 JWT", PROVIDER))?,
        );
        let signing_input = format!("{header}.{payload}");
        let signature = self.signing_key.sign(signing_input.as_bytes());
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ))
    }

    fn cache_get(&self, key: &str) -> Option<Value> {
        self.cache_path
            .as_deref()
            .and_then(|path| Cache::open(path).ok()?.get(key).ok().flatten())
    }

    fn cache_put(&self, key: &str, ttl: i64, value: &Value) {
        if let Some(path) = self.cache_path.as_deref()
            && let Ok(cache) = Cache::open(path)
        {
            let _ = cache.put(key, ttl, value);
        }
    }
}

fn required_env(name: &str) -> Result<String, ToolError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::auth(format!("缺少 {name}"), PROVIDER))
}

fn ensure_qweather_ok(payload: &Value) -> Result<(), ToolError> {
    let code = payload.get("code").and_then(Value::as_str).unwrap_or("200");
    if code == "200" {
        return Ok(());
    }
    let (stable, retryable) = match code {
        "401" | "403" => ("AUTH_MISSING", false),
        "429" => ("RATE_LIMITED", true),
        "204" | "404" => ("NO_RESULTS", false),
        _ => ("UPSTREAM_ERROR", code.starts_with('5')),
    };
    Err(ToolError::new(stable, format!("和风天气返回业务码 {code}"), retryable).provider(PROVIDER))
}

fn urlencoding(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn maps_qweather_business_codes() {
        assert!(ensure_qweather_ok(&json!({"code": "200"})).is_ok());
        assert_eq!(
            ensure_qweather_ok(&json!({"code": "401"}))
                .unwrap_err()
                .code,
            "AUTH_MISSING"
        );
        assert_eq!(
            ensure_qweather_ok(&json!({"code": "429"}))
                .unwrap_err()
                .code,
            "RATE_LIMITED"
        );
    }

    #[tokio::test]
    async fn resolves_location_and_combines_weather() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/geo/v2/city/lookup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code": "200",
                "location": [{"id":"101010100","name":"北京","adm1":"北京市","adm2":"北京","lat":"39.90","lon":"116.41","tz":"Asia/Shanghai"}]
            }))).mount(&server).await;
        Mock::given(method("GET")).and(path("/v7/weather/now"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code":"200","updateTime":"2026-08-03T10:00+08:00","now":{"temp":"30","text":"晴"},"refer":{"sources":["QWeather"]}
            }))).mount(&server).await;
        Mock::given(method("GET")).and(path("/v7/weather/3d"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "code":"200","daily":[{"fxDate":"2026-08-03","textDay":"晴"},{"fxDate":"2026-08-04","textDay":"多云"}]
            }))).mount(&server).await;
        let client = WeatherClient {
            client: Client::new(),
            api_host: server.uri(),
            project_id: "project".into(),
            credential_id: "credential".into(),
            signing_key: SigningKey::from_bytes(&[7_u8; 32]),
            private_key_text: "private-test-value".into(),
            cache_path: None,
        };
        let output = client
            .lookup(WeatherInput {
                location: "北京".into(),
                days: 2,
                include_hourly: false,
            })
            .await
            .unwrap();
        assert_eq!(output.data["location"]["location_id"], "101010100");
        assert_eq!(output.data["now"]["temp"], "30");
        assert_eq!(output.data["daily"].as_array().unwrap().len(), 2);
    }
}
