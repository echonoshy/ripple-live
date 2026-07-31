use std::{path::Path, sync::OnceLock};

use chrono::SecondsFormat;
use chrono_tz::Tz;
use regex::Regex;
use serde_json::{Value, json};

use crate::{
    capabilities::{CliRunner, SkillRegistry},
    context::ContextStore,
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
                "description": "把用户明确要求记住的信息保存到长期上下文。",
                "parameters": {
                    "type": "object",
                    "properties": {"content": {"type": "string"}},
                    "required": ["content"],
                    "additionalProperties": false
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "recall",
                "description": "检索当前会话中保存的长期信息。",
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
    let explicit = EXPLICIT
        .get_or_init(|| Regex::new(r#"(?i)(?:调用|使用)\s*[`'"]?([a-z_]+)[`'"]?\s*工具"#).unwrap());
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
            _ => None,
        };
    }
    if (transcript.contains("记住：")
        || transcript.contains("记住:")
        || transcript.contains("记住，")
        || transcript.contains("记住 "))
        && (transcript.contains("请") || transcript.contains("帮我") || transcript.contains("记住"))
    {
        return Some("remember");
    }
    if ["长期记忆", "查找记忆", "检索记忆", "回忆工具"]
        .iter()
        .any(|needle| transcript.contains(needle))
    {
        return Some("recall");
    }
    None
}

#[derive(Clone)]
pub struct ToolExecutor {
    context: ContextStore,
    registry: SkillRegistry,
    cli: CliRunner,
}

impl ToolExecutor {
    pub fn new(
        context: ContextStore,
        skills_dir: &Path,
        max_output_bytes: usize,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            context,
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
        if let Some(name) = explicit_tool_name(transcript)
            && (is_builtin_tool(name) || self.registry.get(name).is_some())
        {
            return Some(name.to_owned());
        }
        select_forced_tool(transcript).map(str::to_owned)
    }

    pub async fn execute(&self, session_id: &str, name: &str, arguments: &str) -> Value {
        match self.execute_inner(session_id, name, arguments).await {
            Ok(value) => value,
            Err(error) => json!({"ok": false, "error": error.to_string()}),
        }
    }

    async fn execute_inner(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
    ) -> anyhow::Result<Value> {
        let payload: Value = serde_json::from_str(arguments)?;
        match name {
            "get_current_time" => {
                let name = payload
                    .get("timezone")
                    .and_then(Value::as_str)
                    .unwrap_or("Asia/Shanghai");
                let timezone: Tz = name.parse()?;
                let current = chrono::Utc::now().with_timezone(&timezone);
                Ok(json!({
                    "ok": true,
                    "timezone": name,
                    "datetime": current.to_rfc3339_opts(SecondsFormat::Secs, true)
                }))
            }
            "calculate" => {
                let expression = payload
                    .get("expression")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("缺少 expression"))?;
                Ok(json!({"ok": true, "result": safe_calculate(expression)?}))
            }
            "remember" => {
                let content = payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("记忆内容不能为空"))?;
                let memory_id = self.context.remember(session_id, content).await?;
                Ok(json!({"ok": true, "memory_id": memory_id}))
            }
            "recall" => {
                let query = payload.get("query").and_then(Value::as_str).unwrap_or("");
                let memories = self.context.recall(session_id, query, 5).await?;
                Ok(json!({"ok": true, "memories": memories}))
            }
            _ => match self.registry.get(name) {
                Some(tool) => self.cli.execute(tool, &payload).await,
                None => Ok(json!({"ok": false, "error": format!("未知工具: {name}")})),
            },
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

fn is_builtin_tool(name: &str) -> bool {
    matches!(
        name,
        "get_current_time" | "calculate" | "remember" | "recall"
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
    }
}
