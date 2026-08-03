use std::{collections::HashSet, path::Path, sync::OnceLock, time::Duration};

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
                "name": "web_search",
                "description": "使用 DuckDuckGo 在线查询事实摘要和相关网页来源。适用于需要联网、最新信息或外部资料的问题；若没有结果应如实说明。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "简洁、具体的搜索关键词，不要使用“是什么”等完整问句，例如 OpenAI、量子计算"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "最多返回的来源数量，范围 1 到 8",
                            "default": 5,
                            "minimum": 1,
                            "maximum": 8
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }
            }
        }),
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
            "web_search" => Some("web_search"),
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
    if [
        "联网搜索",
        "网上搜索",
        "网页搜索",
        "用 DuckDuckGo",
        "使用 DuckDuckGo",
    ]
    .iter()
    .any(|needle| transcript.contains(needle))
    {
        return Some("web_search");
    }
    None
}

#[derive(Clone)]
pub struct ToolExecutor {
    context: ContextStore,
    registry: SkillRegistry,
    cli: CliRunner,
    client: reqwest::Client,
    search_proxy: Option<String>,
}

impl ToolExecutor {
    pub fn new(
        context: ContextStore,
        skills_dir: &Path,
        max_output_bytes: usize,
        search_proxy: &str,
    ) -> anyhow::Result<Self> {
        let mut client = reqwest::Client::builder();
        let search_proxy =
            (!search_proxy.trim().is_empty()).then(|| search_proxy.trim().to_owned());
        if let Some(proxy) = &search_proxy {
            client = client.proxy(reqwest::Proxy::all(proxy)?);
        }
        Ok(Self {
            context,
            registry: SkillRegistry::load(skills_dir)?,
            cli: CliRunner::new(max_output_bytes),
            client: client.build()?,
            search_proxy,
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
            Err(error) => json!({"ok": false, "error": format!("{error:#}")}),
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
            "web_search" => {
                let query = payload
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|query| !query.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("搜索关键词不能为空"))?;
                if query.chars().count() > 200 {
                    anyhow::bail!("搜索关键词不能超过 200 个字符");
                }
                let max_results = payload
                    .get("max_results")
                    .and_then(Value::as_u64)
                    .unwrap_or(5)
                    .clamp(1, 8) as usize;
                let mut effective_query = query.to_owned();
                let mut response = self.fetch_duckduckgo(&effective_query).await?;
                let simplified = simplify_search_query(query);
                if !duckduckgo_has_results(&response) && simplified != query {
                    effective_query = simplified;
                    response = self.fetch_duckduckgo(&effective_query).await?;
                }
                Ok(format_duckduckgo_results(
                    query,
                    &effective_query,
                    &response,
                    max_results,
                ))
            }
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

    async fn fetch_duckduckgo(&self, query: &str) -> anyhow::Result<Value> {
        let response = self
            .client
            .get("https://api.duckduckgo.com/")
            .query(&[
                ("q", query),
                ("format", "json"),
                ("no_html", "1"),
                ("no_redirect", "1"),
                ("skip_disambig", "0"),
                ("t", "ripple-live"),
            ])
            .header(
                reqwest::header::USER_AGENT,
                "RippleLive/0.2 (DuckDuckGo Instant Answer client)",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .timeout(Duration::from_secs(12))
            .send()
            .await?
            .error_for_status()?;
        let body = response.bytes().await?;
        if !body.is_empty()
            && let Ok(payload) = serde_json::from_slice(&body)
        {
            return Ok(payload);
        }
        self.fetch_duckduckgo_with_curl(query).await
    }

    async fn fetch_duckduckgo_with_curl(&self, query: &str) -> anyhow::Result<Value> {
        let encoded_query = format!("q={query}");
        let mut command = tokio::process::Command::new("curl");
        command.args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "12",
            "--get",
            "https://api.duckduckgo.com/",
            "--data-urlencode",
            &encoded_query,
            "--data",
            "format=json",
            "--data",
            "no_html=1",
            "--data",
            "no_redirect=1",
            "--data",
            "skip_disambig=0",
            "--data",
            "t=ripple-live",
            "--header",
            "User-Agent: RippleLive/0.2 (DuckDuckGo Instant Answer client)",
        ]);
        if let Some(proxy) = &self.search_proxy {
            command.args(["--proxy", proxy]);
        }
        let output = command.output().await?;
        if !output.status.success() {
            anyhow::bail!(
                "DuckDuckGo curl request failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(serde_json::from_slice(&output.stdout)?)
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
        "web_search" | "get_current_time" | "calculate" | "remember" | "recall"
    )
}

fn duckduckgo_has_results(payload: &Value) -> bool {
    ["Answer", "AbstractText"].iter().any(|key| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    }) || payload
        .get("RelatedTopics")
        .and_then(Value::as_array)
        .is_some_and(|topics| !topics.is_empty())
}

fn simplify_search_query(query: &str) -> String {
    let mut simplified = query.trim().trim_matches(['？', '?', '。', '！', '!']);
    for prefix in ["请问", "请搜索", "请查询", "搜索", "查询"] {
        simplified = simplified.strip_prefix(prefix).unwrap_or(simplified).trim();
    }
    for suffix in [
        "是什么",
        "是谁",
        "怎么样",
        "有哪些",
        "的最新消息",
        "最新消息",
    ] {
        simplified = simplified.strip_suffix(suffix).unwrap_or(simplified).trim();
    }
    if simplified.is_empty() {
        query.trim().to_owned()
    } else {
        simplified.to_owned()
    }
}

fn format_duckduckgo_results(
    query: &str,
    effective_query: &str,
    payload: &Value,
    max_results: usize,
) -> Value {
    let answer = payload
        .get("Answer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let abstract_text = payload
        .get("AbstractText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let abstract_url = payload
        .get("AbstractURL")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let abstract_source = payload
        .get("AbstractSource")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();
    if !abstract_text.is_empty() {
        if !abstract_url.is_empty() {
            seen_urls.insert(abstract_url.to_owned());
        }
        results.push(json!({
            "title": payload.get("Heading").and_then(Value::as_str).unwrap_or(abstract_source),
            "snippet": abstract_text,
            "url": abstract_url,
            "source": abstract_source
        }));
    }
    if let Some(topics) = payload.get("RelatedTopics").and_then(Value::as_array) {
        collect_related_topics(topics, &mut results, &mut seen_urls, max_results);
    }
    results.truncate(max_results);
    let result_count = results.len();
    json!({
        "ok": true,
        "provider": "duckduckgo_instant_answer",
        "query": query,
        "effective_query": effective_query,
        "answer": answer,
        "results": results,
        "result_count": result_count,
        "limitations": "DuckDuckGo Instant Answer 不是完整网页搜索 API；部分查询可能没有摘要或相关来源。"
    })
}

fn collect_related_topics(
    topics: &[Value],
    output: &mut Vec<Value>,
    seen_urls: &mut HashSet<String>,
    limit: usize,
) {
    for topic in topics {
        if output.len() >= limit {
            return;
        }
        if let Some(nested) = topic.get("Topics").and_then(Value::as_array) {
            collect_related_topics(nested, output, seen_urls, limit);
            continue;
        }
        let text = topic
            .get("Text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let url = topic
            .get("FirstURL")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if text.is_empty() || url.is_empty() || !seen_urls.insert(url.to_owned()) {
            continue;
        }
        output.push(json!({"title": text, "snippet": text, "url": url}));
    }
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
        assert_eq!(select_forced_tool("请联网搜索 OpenAI"), Some("web_search"));
    }

    #[test]
    fn formats_duckduckgo_instant_answers() {
        let payload = json!({
            "Heading": "Ripple",
            "AbstractText": "Ripple is a test abstract.",
            "AbstractURL": "https://example.com/ripple",
            "AbstractSource": "Example",
            "Answer": "",
            "RelatedTopics": [
                {"Text": "First related result", "FirstURL": "https://example.com/first"},
                {"Name": "Nested", "Topics": [
                    {"Text": "Second related result", "FirstURL": "https://example.com/second"}
                ]}
            ]
        });
        let result = format_duckduckgo_results("ripple 是什么", "ripple", &payload, 2);
        assert_eq!(result["ok"], json!(true));
        assert_eq!(result["provider"], json!("duckduckgo_instant_answer"));
        assert_eq!(result["result_count"], json!(2));
        assert_eq!(result["effective_query"], json!("ripple"));
        assert_eq!(result["results"][0]["source"], json!("Example"));
        assert_eq!(
            result["results"][1]["url"],
            json!("https://example.com/first")
        );
        assert_eq!(simplify_search_query("OpenAI 是什么？"), "OpenAI");
        assert_eq!(simplify_search_query("请查询量子计算"), "量子计算");
    }
}
