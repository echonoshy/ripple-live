use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct RegisteredTool {
    pub skill_name: String,
    pub skill_description: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub command: Vec<String>,
    pub working_dir: PathBuf,
    pub timeout_ms: u64,
    pub rate_limit_per_minute: u32,
    pub env_allowlist: Vec<String>,
    pub confirmation: Confirmation,
}

impl RegisteredTool {
    pub fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": format!("{} Skill: {}", self.description, self.skill_description),
                "parameters": self.input_schema
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    #[default]
    Never,
    Always,
}

#[derive(Debug, Deserialize)]
struct ToolsManifest {
    tools: Vec<ToolManifest>,
}

#[derive(Debug, Deserialize)]
struct ToolManifest {
    name: String,
    description: String,
    input_schema: Value,
    command: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    rate_limit_per_minute: u32,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    confirmation: Confirmation,
}

fn default_timeout_ms() -> u64 {
    15_000
}

#[derive(Clone, Default)]
pub struct SkillRegistry {
    tools: Arc<BTreeMap<String, RegisteredTool>>,
}

impl SkillRegistry {
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        if !root.exists() {
            return Ok(Self::default());
        }
        let mut tools = BTreeMap::new();
        for entry in std::fs::read_dir(root)
            .with_context(|| format!("无法读取 Skill 目录 {}", root.display()))?
        {
            let path = entry?.path();
            if !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let (skill_name, skill_description) = parse_skill_metadata(&path.join("SKILL.md"))?;
            let manifest_path = path.join("tools.json");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest: ToolsManifest =
                serde_json::from_slice(&std::fs::read(&manifest_path)?)
                    .with_context(|| format!("无效的工具清单 {}", manifest_path.display()))?;
            for tool in manifest.tools {
                validate_tool(&tool, &manifest_path)?;
                let registered = RegisteredTool {
                    skill_name: skill_name.clone(),
                    skill_description: skill_description.clone(),
                    name: tool.name.clone(),
                    description: tool.description,
                    input_schema: tool.input_schema,
                    command: tool.command,
                    working_dir: path.clone(),
                    timeout_ms: tool.timeout_ms.clamp(100, 300_000),
                    rate_limit_per_minute: tool.rate_limit_per_minute.min(600),
                    env_allowlist: tool.env,
                    confirmation: tool.confirmation,
                };
                if tools.insert(tool.name.clone(), registered).is_some() {
                    anyhow::bail!("重复的工具名称: {}", tool.name);
                }
            }
        }
        Ok(Self {
            tools: Arc::new(tools),
        })
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools.values().map(RegisteredTool::schema).collect()
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

fn parse_skill_metadata(path: &Path) -> anyhow::Result<(String, String)> {
    let content = std::fs::read_to_string(path)?;
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        anyhow::bail!("{} 缺少 YAML frontmatter", path.display());
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value.trim()).to_owned());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(unquote(value.trim()).to_owned());
        }
    }
    Ok((
        name.filter(|value| !value.is_empty())
            .context("Skill 缺少 name")?,
        description
            .filter(|value| !value.is_empty())
            .context("Skill 缺少 description")?,
    ))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn validate_tool(tool: &ToolManifest, path: &Path) -> anyhow::Result<()> {
    let valid_name = Regex::new(r"^[a-z][a-z0-9_]{0,63}$").unwrap();
    if !valid_name.is_match(&tool.name) {
        anyhow::bail!("{} 中的工具名无效: {}", path.display(), tool.name);
    }
    if tool.description.trim().is_empty() || tool.command.is_empty() {
        anyhow::bail!("工具 {} 缺少 description 或 command", tool.name);
    }
    if tool.input_schema.get("type").and_then(Value::as_str) != Some("object") {
        anyhow::bail!("工具 {} 的 input_schema 必须是 object", tool.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_skill_tool_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let skill = directory.path().join("demo");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo skill\n---\n",
        )
        .unwrap();
        std::fs::write(
            skill.join("tools.json"),
            r#"{"tools":[{"name":"demo_query","description":"query","input_schema":{"type":"object"},"command":["demo"]}]}"#,
        )
        .unwrap();

        let registry = SkillRegistry::load(directory.path()).unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get("demo_query").is_some());
        assert_eq!(registry.get("demo_query").unwrap().rate_limit_per_minute, 0);
    }
}
