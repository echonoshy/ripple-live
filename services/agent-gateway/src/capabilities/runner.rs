use std::{
    collections::{HashMap, HashSet, VecDeque},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
};

use super::{RegisteredTool, registry::Confirmation};

type RateLimitBuckets = HashMap<(String, String), VecDeque<Instant>>;

#[derive(Clone)]
pub struct CliRunner {
    max_output_bytes: usize,
    rate_limits: Arc<Mutex<RateLimitBuckets>>,
}

impl CliRunner {
    pub fn new(max_output_bytes: usize) -> Self {
        Self {
            max_output_bytes: max_output_bytes.clamp(1_024, 4 * 1024 * 1024),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn execute(
        &self,
        tool: &RegisteredTool,
        arguments: &Value,
        subject: &str,
    ) -> anyhow::Result<Value> {
        validate_arguments(&tool.input_schema, arguments)?;
        if tool.confirmation == Confirmation::Always {
            return Ok(json!({
                "ok": false,
                "needs_confirmation": true,
                "message": "这个工具需要用户明确确认后才能执行"
            }));
        }
        if !self.admit(tool, subject).await {
            return Ok(json!({
                "ok": false,
                "error": {
                    "code": "RATE_LIMITED",
                    "message": "这个工具调用过于频繁，请稍后再试",
                    "retryable": true
                },
                "meta": {"provider": "gateway"}
            }));
        }

        let program = tool
            .command
            .first()
            .ok_or_else(|| anyhow::anyhow!("工具没有可执行命令"))?;
        let mut command = Command::new(program);
        command
            .args(&tool.command[1..])
            .current_dir(&tool.working_dir)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in &tool.env_allowlist {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }

        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let input = serde_json::to_vec(arguments)?;
        stdin.write_all(&input).await?;
        stdin.shutdown().await?;
        drop(stdin);

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let limit = self.max_output_bytes as u64 + 1;
        let output = async move {
            let stdout_task = tokio::spawn(async move {
                let mut bytes = Vec::new();
                stdout.take(limit).read_to_end(&mut bytes).await?;
                anyhow::Ok(bytes)
            });
            let stderr_task = tokio::spawn(async move {
                let mut bytes = Vec::new();
                stderr.take(limit).read_to_end(&mut bytes).await?;
                anyhow::Ok(bytes)
            });
            let status = child.wait().await?;
            let stdout = stdout_task.await??;
            let stderr = stderr_task.await??;
            anyhow::Ok((status, stdout, stderr))
        };
        let (status, stdout, stderr) =
            tokio::time::timeout(Duration::from_millis(tool.timeout_ms), output)
                .await
                .map_err(|_| anyhow::anyhow!("工具执行超过 {} ms", tool.timeout_ms))??;

        if stdout.len() > self.max_output_bytes || stderr.len() > self.max_output_bytes {
            anyhow::bail!("工具输出超过 {} bytes", self.max_output_bytes);
        }
        if !status.success() {
            anyhow::bail!(
                "工具退出码 {:?}: {}",
                status.code(),
                String::from_utf8_lossy(&stderr).trim()
            );
        }
        if stdout.is_empty() {
            anyhow::bail!("工具没有返回 JSON");
        }
        Ok(serde_json::from_slice(&stdout)?)
    }

    async fn admit(&self, tool: &RegisteredTool, subject: &str) -> bool {
        let limit = tool.rate_limit_per_minute;
        if limit == 0 {
            return true;
        }
        let now = Instant::now();
        let key = (subject.to_owned(), tool.name.clone());
        let mut limits = self.rate_limits.lock().await;
        let calls = limits.entry(key).or_default();
        while calls
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            calls.pop_front();
        }
        if calls.len() >= limit as usize {
            return false;
        }
        calls.push_back(now);
        true
    }
}

fn validate_arguments(schema: &Value, arguments: &Value) -> anyhow::Result<()> {
    let object = arguments
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("工具参数必须是 JSON object"))?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for name in required {
        if !object.contains_key(name) {
            anyhow::bail!("缺少必填参数: {name}");
        }
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        for name in object.keys() {
            if !properties.contains_key(name) {
                anyhow::bail!("不允许的参数: {name}");
            }
        }
    }
    for (name, value) in object {
        if let Some(property_schema) = properties.get(name) {
            validate_value(name, property_schema, value)?;
        }
    }
    Ok(())
}

fn validate_value(name: &str, schema: &Value, value: &Value) -> anyhow::Result<()> {
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        anyhow::bail!("参数 {name} 不在允许值范围内");
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("参数 {name} 必须是 string"))?;
            let length = text.chars().count() as u64;
            if schema
                .get("minLength")
                .and_then(Value::as_u64)
                .is_some_and(|min| length < min)
            {
                anyhow::bail!("参数 {name} 长度不足");
            }
            if schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|max| length > max)
            {
                anyhow::bail!("参数 {name} 长度超限");
            }
        }
        Some("integer") => {
            let number = value
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("参数 {name} 必须是 integer"))?;
            if schema
                .get("minimum")
                .and_then(Value::as_i64)
                .is_some_and(|min| number < min)
            {
                anyhow::bail!("参数 {name} 小于最小值");
            }
            if schema
                .get("maximum")
                .and_then(Value::as_i64)
                .is_some_and(|max| number > max)
            {
                anyhow::bail!("参数 {name} 大于最大值");
            }
        }
        Some("number") if !value.is_number() => anyhow::bail!("参数 {name} 必须是 number"),
        Some("boolean") if !value.is_boolean() => anyhow::bail!("参数 {name} 必须是 boolean"),
        Some("array") if !value.is_array() => anyhow::bail!("参数 {name} 必须是 array"),
        Some("object") if !value.is_object() => anyhow::bail!("参数 {name} 必须是 object"),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::capabilities::SkillRegistry;

    #[tokio::test]
    async fn executes_the_example_skill_as_json_in_json_out() {
        let skills_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let registry = SkillRegistry::load(&skills_dir).unwrap();
        let tool = registry.get("system_info").unwrap();
        let result = CliRunner::new(64 * 1024)
            .execute(tool, &json!({}), "test-user")
            .await
            .unwrap();
        assert_eq!(result["ok"], json!(true));
        assert!(result["data"]["kernel"].is_string());
    }

    #[tokio::test]
    async fn rate_limits_each_user_and_tool_independently() {
        let skills_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let registry = SkillRegistry::load(&skills_dir).unwrap();
        let mut tool = registry.get("system_info").unwrap().clone();
        tool.rate_limit_per_minute = 1;
        let runner = CliRunner::new(64 * 1024);

        let first = runner.execute(&tool, &json!({}), "user-a").await.unwrap();
        let limited = runner.execute(&tool, &json!({}), "user-a").await.unwrap();
        let other_user = runner.execute(&tool, &json!({}), "user-b").await.unwrap();

        assert_eq!(first["ok"], json!(true));
        assert_eq!(limited["error"]["code"], json!("RATE_LIMITED"));
        assert_eq!(other_user["ok"], json!(true));
    }

    #[test]
    fn validates_types_ranges_and_enums() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "minimum": 1, "maximum": 8},
                "topic": {"type": "string", "enum": ["general", "news"], "maxLength": 20}
            },
            "required": ["count", "topic"],
            "additionalProperties": false
        });
        assert!(validate_arguments(&schema, &json!({"count": 5, "topic": "news"})).is_ok());
        assert!(validate_arguments(&schema, &json!({"count": 9, "topic": "news"})).is_err());
        assert!(validate_arguments(&schema, &json!({"count": 5, "topic": "other"})).is_err());
        assert!(validate_arguments(&schema, &json!({"count": "5", "topic": "news"})).is_err());
    }
}
