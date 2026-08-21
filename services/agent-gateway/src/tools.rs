use std::{path::Path, sync::OnceLock};

use chrono::SecondsFormat;
use chrono_tz::Tz;
use regex::Regex;
use serde_json::{Value, json};

use crate::{
    capabilities::{CliRunner, SkillRegistry},
    memory::{CreateMemoryRequest, CreateTodoRequest, MemoryArtifact, MemoryService, TodoRecord},
    protocol::VideoFrame,
};

fn builtin_schemas() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "get_current_time",
                "description": "查询指定时区的当前日期和时间。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "timezone": {
                            "type": "string",
                            "description": "IANA 时区，例如 Asia/Shanghai",
                            "default": "Asia/Shanghai"
                        }
                    },
                    "required": ["timezone"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "calculate",
                "description": "计算一个只包含数字和常见数学运算符的表达式。",
                "parameters": {
                    "type": "object",
                    "properties": {"expression": {"type": "string"}},
                    "required": ["expression"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "remember",
                "description": "把用户明确要求记住的信息和当前画面保存到用户的长期记忆。只有用户明确要求时才能调用。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "用户希望记住的核心信息"},
                        "visual_summary": {"type": "string", "description": "根据当前画面生成的客观、可检索描述；没有画面时留空"}
                    },
                    "required": ["content"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "create_todo",
                "description": "仅在用户明确要求把当前内容做成待办或提醒时调用。会保存当前画面作为可回看的证据，并创建一个待办。用户没有说明确时间时不要设置 due_at；有明确提醒时间时，due_at 必须是带时区的 RFC3339 时间。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "简短、可执行的待办标题"},
                        "visual_summary": {"type": "string", "description": "对当前画面和待办来源的一句话摘要"},
                        "due_at": {"type": "string", "description": "明确提醒时间的 RFC3339 格式，例如 2026-08-04T10:00:00+08:00；没有明确时间则省略"}
                    },
                    "required": ["title"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_todos",
                "description": "查询当前用户的待办事项。用户询问有哪些、多少个或之前创建了什么待办时必须调用；completed 省略时只返回未完成待办。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "completed": {
                            "type": "boolean",
                            "description": "false 查询未完成待办，true 查询已完成待办；省略时为 false"
                        }
                    },
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "recall",
                "description": "从当前用户所有会话保存的长期记忆中检索信息，并返回相关原始画面。query 应提炼为简洁关键词。",
                "parameters": {
                    "type": "object",
                    "properties": {"query": {"type": "string", "default": ""}},
                    "required": ["query"],
                    "additionalProperties": false
                }
            }
        }),
    ]
}

pub fn select_forced_tool(transcript: &str) -> Option<&'static str> {
    static EXPLICIT: OnceLock<Regex> = OnceLock::new();
    let explicit = EXPLICIT.get_or_init(|| {
        Regex::new(r#"(?i)(?:调用|使用)\s*[`'\"]?([a-z_]+)[`'\"]?\s*工具"#).unwrap()
    });
    if let Some(name) = explicit
        .captures(transcript)
        .and_then(|capture| capture.get(1))
        .map(|item| item.as_str())
    {
        return match name {
            "get_current_time" => Some("get_current_time"),
            "calculate" => Some("calculate"),
            "remember" => Some("remember"),
            "recall" => Some("recall"),
            "create_todo" => Some("create_todo"),
            "list_todos" => Some("list_todos"),
            "web_search" => Some("web_search"),
            _ => None,
        };
    }
    if is_memory_search_request(transcript) {
        return Some("recall");
    }
    if [
        "帮我记住",
        "请记住",
        "记住这个",
        "记一下这个",
        "记录一下",
        "记录这个",
        "保存一下",
        "保存这个画面",
        "记住这个位置",
        "记住：",
        "记住:",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
    {
        return Some("remember");
    }
    if [
        "做成待办",
        "创建待办",
        "加个待办",
        "记个待办",
        "加入待办",
        "提醒我",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
    {
        return Some("create_todo");
    }
    if asks_about_todos(transcript) {
        return Some("list_todos");
    }
    if transcript.contains("天气") && Regex::new("[省市区县镇乡村]").unwrap().is_match(transcript)
    {
        return Some("weather_lookup");
    }
    if [
        "联网搜索",
        "网上搜索",
        "网页搜索",
        "联网查一下",
        "用 Tavily",
        "使用 Tavily",
        "最新消息",
        "最新新闻",
        "实时天气",
        "实时股价",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
    {
        return Some("web_search");
    }
    None
}

fn is_memory_search_request(transcript: &str) -> bool {
    [
        "长期记忆",
        "查找记忆",
        "检索记忆",
        "搜索记忆",
        "记忆中",
        "记忆里的",
        "记忆图库",
        "图库里的",
        "存储的图片",
        "保存的图片",
        "回忆工具",
        "你还记得",
        "我上次放",
        "我之前放",
        "我把它放哪",
        "上次放哪里",
        "之前放哪里",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
}

#[derive(Clone)]
pub struct ToolExecutor {
    memories: MemoryService,
    registry: SkillRegistry,
    cli: CliRunner,
}

impl ToolExecutor {
    pub fn new(
        memories: MemoryService,
        skills_dir: &Path,
        max_output_bytes: usize,
        _search_proxy: &str,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            memories,
            registry: SkillRegistry::load(skills_dir)?,
            cli: CliRunner::new(max_output_bytes),
        })
    }

    pub fn schemas(&self) -> Vec<Value> {
        let mut schemas = builtin_schemas();
        schemas.extend(self.registry.schemas());
        schemas
    }

    pub fn external_tool_count(&self) -> usize {
        self.registry.len()
    }

    pub fn select_forced_tool(&self, transcript: &str) -> Option<String> {
        self.forced_route(transcript).map(|route| route.name)
    }

    pub fn forced_route(&self, transcript: &str) -> Option<ToolRoute> {
        if let Some(name) = explicit_tool_name(transcript)
            && (is_builtin_tool(name) || self.registry.get(name).is_some())
        {
            return Some(ToolRoute {
                name: name.to_owned(),
                reason: "explicit_tool",
            });
        }
        let selected = select_forced_tool(transcript)
            .filter(|name| is_builtin_tool(name) || self.registry.get(name).is_some());
        if selected == Some("create_todo") && mentions_relative_time(transcript) {
            return Some(ToolRoute {
                name: "get_current_time".to_owned(),
                reason: "relative_todo_time",
            });
        }
        selected.map(|name| ToolRoute {
            name: name.to_owned(),
            reason: match name {
                "recall" => "memory_scope",
                "list_todos" => "todo_query",
                "create_todo" => "todo_create",
                "remember" => "memory_save",
                "web_search" => "explicit_web_scope",
                "weather_lookup" => "weather_query",
                _ => "keyword_rule",
            },
        })
    }

    pub async fn execute(
        &self,
        execution: &ToolExecutionContext,
        name: &str,
        arguments: &str,
    ) -> ToolOutcome {
        match self.execute_inner(execution, name, arguments).await {
            Ok(outcome) => outcome,
            Err(error) => ToolOutcome::value(json!({"ok": false, "error": format!("{error:#}")})),
        }
    }

    async fn execute_inner(
        &self,
        execution: &ToolExecutionContext,
        name: &str,
        arguments: &str,
    ) -> anyhow::Result<ToolOutcome> {
        let payload: Value = serde_json::from_str(arguments)?;
        match name {
            "get_current_time" => {
                let name = payload
                    .get("timezone")
                    .and_then(Value::as_str)
                    .unwrap_or("Asia/Shanghai");
                let timezone: Tz = name.parse()?;
                let current = chrono::Utc::now().with_timezone(&timezone);
                Ok(ToolOutcome::value(json!({
                    "ok": true,
                    "timezone": name,
                    "datetime": current.to_rfc3339_opts(SecondsFormat::Secs, true)
                })))
            }
            "calculate" => {
                let expression = payload
                    .get("expression")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("缺少 expression"))?;
                Ok(ToolOutcome::value(
                    json!({"ok": true, "result": safe_calculate(expression)?}),
                ))
            }
            "remember" => {
                let content = payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("记忆内容不能为空"))?;
                let visual_summary = payload
                    .get("visual_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let memory = self
                    .memories
                    .create(CreateMemoryRequest {
                        user_id: execution.user_id.clone(),
                        conversation_id: execution.conversation_id.clone(),
                        source_turn_id: execution.user_turn_id,
                        response_id: execution.response_id.clone(),
                        tool_call_id: execution.tool_call_id.clone(),
                        user_note: content.to_owned(),
                        visual_summary: visual_summary.to_owned(),
                        frames: execution.frames.clone(),
                    })
                    .await?;
                let artifacts = memory.memory.cover.clone().into_iter().collect::<Vec<_>>();
                Ok(ToolOutcome {
                    value: json!({
                        "ok": true,
                        "memory": memory.memory,
                        "saved_frame_count": memory.assets.len()
                    }),
                    artifacts,
                })
            }
            "create_todo" => {
                let title = payload
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("待办标题不能为空"))?;
                let due_at = payload
                    .get("due_at")
                    .and_then(Value::as_str)
                    .map(parse_due_at)
                    .transpose()?;
                let todo = self
                    .memories
                    .create_todo(CreateTodoRequest {
                        user_id: execution.user_id.clone(),
                        conversation_id: execution.conversation_id.clone(),
                        source_turn_id: execution.user_turn_id,
                        response_id: execution.response_id.clone(),
                        tool_call_id: execution.tool_call_id.clone(),
                        title: title.to_owned(),
                        visual_summary: payload
                            .get("visual_summary")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        due_at,
                        frames: execution.frames.clone(),
                    })
                    .await?;
                let artifacts = todo.cover.clone().into_iter().collect();
                Ok(ToolOutcome {
                    value: json!({"ok": true, "todo": todo}),
                    artifacts,
                })
            }
            "list_todos" => {
                let completed = payload
                    .get("completed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let todos = self
                    .memories
                    .list_todos(&execution.user_id, Some(completed), 100)
                    .await?;
                let artifacts = todo_artifacts(&todos);
                Ok(ToolOutcome {
                    value: json!({
                        "ok": true,
                        "completed": completed,
                        "count": todos.len(),
                        "todos": todos
                    }),
                    artifacts,
                })
            }
            "recall" => {
                let query = payload.get("query").and_then(Value::as_str).unwrap_or("");
                let memories = self
                    .memories
                    .recall(
                        &execution.user_id,
                        Some(&execution.conversation_id),
                        query,
                        5,
                    )
                    .await?;
                let artifacts = memories
                    .iter()
                    .filter_map(|memory| memory.memory.cover.clone())
                    .collect();
                Ok(ToolOutcome {
                    value: json!({"ok": true, "memories": memories}),
                    artifacts,
                })
            }
            _ => match self.registry.get(name) {
                Some(tool) => Ok(ToolOutcome::value(
                    self.cli.execute(tool, &payload, &execution.user_id).await?,
                )),
                None => Ok(ToolOutcome::value(
                    json!({"ok": false, "error": format!("未知工具: {name}")}),
                )),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolExecutionContext {
    pub user_id: String,
    pub conversation_id: String,
    pub user_turn_id: i64,
    pub response_id: String,
    pub tool_call_id: String,
    pub transcript: String,
    pub frames: Vec<VideoFrame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRoute {
    pub name: String,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct ToolOutcome {
    pub value: Value,
    pub artifacts: Vec<MemoryArtifact>,
}

impl ToolOutcome {
    fn value(value: Value) -> Self {
        Self {
            value,
            artifacts: Vec::new(),
        }
    }
}

fn explicit_tool_name(transcript: &str) -> Option<&str> {
    static EXPLICIT: OnceLock<Regex> = OnceLock::new();
    EXPLICIT
        .get_or_init(|| {
            Regex::new(r#"(?i)(?:调用|使用)\s*[`'"]?([a-z_][a-z0-9_]*)[`'"]?\s*工具"#).unwrap()
        })
        .captures(transcript)
        .and_then(|capture| capture.get(1))
        .map(|item| item.as_str())
}

fn asks_about_todos(transcript: &str) -> bool {
    let mentions_todo = transcript.contains("待办") || transcript.contains("代办");
    mentions_todo
        && [
            "哪些",
            "多少",
            "几个",
            "什么",
            "有没有",
            "还有",
            "查看",
            "查询",
            "查一下",
            "检查",
            "列出",
            "列表",
            "之前",
            "当前",
            "现在",
            "我的",
        ]
        .iter()
        .any(|needle| transcript.contains(needle))
}

fn todo_artifacts(todos: &[TodoRecord]) -> Vec<MemoryArtifact> {
    todos.iter().filter_map(|todo| todo.cover.clone()).collect()
}

fn mentions_relative_time(transcript: &str) -> bool {
    [
        "今天",
        "明天",
        "后天",
        "今晚",
        "早上",
        "上午",
        "中午",
        "下午",
        "晚上",
        "下班",
        "周末",
        "下周",
        "下个月",
        "一会",
        "稍后",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
}

fn parse_due_at(value: &str) -> anyhow::Result<f64> {
    let due_at = chrono::DateTime::parse_from_rfc3339(value.trim())?.timestamp() as f64;
    if due_at < chrono::Utc::now().timestamp() as f64 - 60.0 {
        anyhow::bail!("提醒时间不能早于当前时间");
    }
    Ok(due_at)
}

fn is_builtin_tool(name: &str) -> bool {
    matches!(
        name,
        "get_current_time" | "calculate" | "remember" | "recall" | "create_todo" | "list_todos"
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Pi,
    E,
    Plus,
    Minus,
    Multiply,
    Divide,
    FloorDivide,
    Modulo,
    Power,
    LeftParen,
    RightParen,
}

pub fn safe_calculate(expression: &str) -> anyhow::Result<Value> {
    if expression.len() > 200 {
        anyhow::bail!("表达式过长");
    }
    let tokens = tokenize(expression)?;
    let mut parser = Parser { tokens, index: 0 };
    let result = parser.expression()?;
    if parser.index != parser.tokens.len() {
        anyhow::bail!("表达式包含不允许的内容");
    }
    if !result.is_finite() {
        anyhow::bail!("结果不是有限数值");
    }
    if result.fract() == 0.0 && result >= i64::MIN as f64 && result <= i64::MAX as f64 {
        Ok(json!(result as i64))
    } else {
        Ok(json!(result))
    }
}

fn tokenize(expression: &str) -> anyhow::Result<Vec<Token>> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    let mut tokens = Vec::new();
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'0'..=b'9' | b'.' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_digit()
                        || bytes[index] == b'.'
                        || bytes[index] == b'e'
                        || bytes[index] == b'E'
                        || ((bytes[index] == b'+' || bytes[index] == b'-')
                            && matches!(bytes[index - 1], b'e' | b'E')))
                {
                    index += 1;
                }
                let number = expression[start..index].parse::<f64>()?;
                tokens.push(Token::Number(number));
            }
            b'a'..=b'z' | b'A'..=b'Z' => {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                tokens.push(match &expression[start..index] {
                    "pi" => Token::Pi,
                    "e" => Token::E,
                    _ => anyhow::bail!("表达式包含不允许的名称"),
                });
            }
            b'+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            b'-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            b'*' if bytes.get(index + 1) == Some(&b'*') => {
                tokens.push(Token::Power);
                index += 2;
            }
            b'*' => {
                tokens.push(Token::Multiply);
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                tokens.push(Token::FloorDivide);
                index += 2;
            }
            b'/' => {
                tokens.push(Token::Divide);
                index += 1;
            }
            b'%' => {
                tokens.push(Token::Modulo);
                index += 1;
            }
            b'(' => {
                tokens.push(Token::LeftParen);
                index += 1;
            }
            b')' => {
                tokens.push(Token::RightParen);
                index += 1;
            }
            _ => anyhow::bail!("表达式包含不允许的内容"),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn expression(&mut self) -> anyhow::Result<f64> {
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.index += 1;
                    value += self.term()?;
                }
                Some(Token::Minus) => {
                    self.index += 1;
                    value -= self.term()?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn term(&mut self) -> anyhow::Result<f64> {
        let mut value = self.unary()?;
        loop {
            match self.peek() {
                Some(Token::Multiply) => {
                    self.index += 1;
                    value *= self.unary()?;
                }
                Some(Token::Divide) => {
                    self.index += 1;
                    value /= self.unary()?;
                }
                Some(Token::FloorDivide) => {
                    self.index += 1;
                    value = (value / self.unary()?).floor();
                }
                Some(Token::Modulo) => {
                    self.index += 1;
                    value %= self.unary()?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn unary(&mut self) -> anyhow::Result<f64> {
        match self.peek() {
            Some(Token::Plus) => {
                self.index += 1;
                self.unary()
            }
            Some(Token::Minus) => {
                self.index += 1;
                Ok(-self.unary()?)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> anyhow::Result<f64> {
        let value = self.primary()?;
        if self.peek() == Some(Token::Power) {
            self.index += 1;
            let exponent = self.unary()?;
            if exponent.abs() > 12.0 {
                anyhow::bail!("指数过大");
            }
            return Ok(value.powf(exponent));
        }
        Ok(value)
    }

    fn primary(&mut self) -> anyhow::Result<f64> {
        let token = self
            .tokens
            .get(self.index)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("表达式不完整"))?;
        self.index += 1;
        match token {
            Token::Number(number) => Ok(number),
            Token::Pi => Ok(std::f64::consts::PI),
            Token::E => Ok(std::f64::consts::E),
            Token::LeftParen => {
                let value = self.expression()?;
                if self.peek() != Some(Token::RightParen) {
                    anyhow::bail!("括号不匹配");
                }
                self.index += 1;
                Ok(value)
            }
            _ => anyhow::bail!("表达式包含不允许的内容"),
        }
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_safely() {
        assert_eq!(safe_calculate("(2 + 3) * 4").unwrap(), json!(20));
        assert_eq!(safe_calculate("2**3**2").unwrap(), json!(512));
        assert!(safe_calculate("__import__('os')").is_err());
        assert!(safe_calculate("2**13").is_err());
    }

    #[test]
    fn routes_explicit_tools() {
        assert_eq!(
            select_forced_tool("请调用 calculate 工具计算"),
            Some("calculate")
        );
        assert_eq!(select_forced_tool("请记住：我喜欢乌龙茶"), Some("remember"));
        assert_eq!(select_forced_tool("记一下这个位置"), Some("remember"));
        assert_eq!(select_forced_tool("把这个记录一下"), Some("remember"));
        assert_eq!(select_forced_tool("我上次放哪里了"), Some("recall"));
        assert_eq!(
            select_forced_tool("你再帮我搜索一下记忆中有关于芦荟胶相关的图片"),
            Some("recall")
        );
        assert_eq!(select_forced_tool("帮我搜索一下芦荟胶"), None);
        assert_eq!(
            select_forced_tool("联网搜索芦荟胶的用法"),
            Some("web_search")
        );
        assert_eq!(select_forced_tool("我有哪些待办"), Some("list_todos"));
        assert_eq!(
            select_forced_tool("帮我检查一下，我还有哪些代办事项"),
            Some("list_todos")
        );
        assert_eq!(
            select_forced_tool("我之前让你创建的是什么待办"),
            Some("list_todos")
        );
        assert_eq!(select_forced_tool("请联网搜索 OpenAI"), Some("web_search"));
        assert_eq!(
            select_forced_tool("帮我查一下明天徐汇区的天气。"),
            Some("weather_lookup")
        );
        assert!(is_builtin_tool("create_todo"));
        assert!(is_builtin_tool("list_todos"));
    }

    #[test]
    fn does_not_force_weather_lookup_without_a_location() {
        assert_eq!(select_forced_tool("今天天气如何？"), None);
        assert_eq!(
            select_forced_tool("今天北京市朝阳区天气如何？"),
            Some("weather_lookup")
        );
    }

    #[test]
    fn returns_todo_cover_as_response_artifact() {
        let cover = MemoryArtifact {
            id: "asset_1".to_owned(),
            kind: "image".to_owned(),
            memory_id: "todo_1".to_owned(),
            todo_id: Some("todo_1".to_owned()),
            caption: "买芦荟胶".to_owned(),
            content_url: "/v1/assets/asset_1/content".to_owned(),
        };
        let artifacts = todo_artifacts(&[TodoRecord {
            id: "todo_1".to_owned(),
            memory_id: None,
            title: "买芦荟胶".to_owned(),
            visual_summary: String::new(),
            due_at: None,
            completed_at: None,
            created_at: 0.0,
            cover: Some(cover.clone()),
        }]);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, cover.id);
    }

    #[test]
    fn recognizes_relative_todo_times() {
        assert!(mentions_relative_time("提醒我明天下班买抽纸"));
        assert!(mentions_relative_time("把这个做成待办，周末处理"));
        assert!(!mentions_relative_time("把这个做成待办"));
    }
}
