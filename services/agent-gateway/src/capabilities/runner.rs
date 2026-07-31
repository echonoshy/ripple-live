use std::{collections::HashSet, process::Stdio, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

use super::{RegisteredTool, registry::Confirmation};

#[derive(Clone)]
pub struct CliRunner {
    max_output_bytes: usize,
}

impl CliRunner {
    pub fn new(max_output_bytes: usize) -> Self {
        Self {
            max_output_bytes: max_output_bytes.clamp(1_024, 4 * 1024 * 1024),
        }
    }

    pub async fn execute(&self, tool: &RegisteredTool, arguments: &Value) -> anyhow::Result<Value> {
        validate_arguments(&tool.input_schema, arguments)?;
        if tool.confirmation == Confirmation::Always {
            return Ok(json!({
                "ok": false,
                "needs_confirmation": true,
                "message": "这个工具需要用户明确确认后才能执行"
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
            .execute(tool, &json!({}))
            .await
            .unwrap();
        assert_eq!(result["ok"], json!(true));
        assert!(result["data"]["kernel"].is_string());
    }
}
