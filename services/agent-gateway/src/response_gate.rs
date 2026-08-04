use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapters::ResponsesOutput;

pub const GATE_INSTRUCTIONS: &str = "你是 Ripple Live 的回复判断器。判断当前语音是否在对助手说话或是否是对近期助手回复的自然追问。明确提问、指令和上下文追问使用 respond；与他人的背景对话、无意义语音使用 ignore。你必须调用 decide_response 一次，不得直接输出文本。";

pub fn gate_tool_schema() -> Value {
    json!({
        "type": "function",
        "name": "decide_response",
        "description": "判断当前语音是否需要助手回复",
        "parameters": {
            "type": "object",
            "properties": {
                "decision": {"type": "string", "enum": ["respond", "ignore"]},
                "reason": {"type": "string"}
            },
            "required": ["decision", "reason"],
            "additionalProperties": false
        }
    })
}

pub fn build_gate_input(
    history: &[Value],
    transcript: &str,
    assistant_just_replied: bool,
) -> Vec<Value> {
    let recent = history
        .iter()
        .filter_map(|item| {
            Some(format!(
                "{}: {}",
                item.get("role")?.as_str()?,
                item.get("content")?.as_str()?
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");
    vec![json!({
        "role": "user",
        "content": [{
            "type": "input_text",
            "text": format!(
                "recent_conversation:\n{recent}\nassistant_just_replied: {assistant_just_replied}\ncurrent_transcript: {transcript}"
            )
        }]
    })]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Respond,
    Ignore,
}

impl GateDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Respond => "respond",
            Self::Ignore => "ignore",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GateOutcome {
    pub decision: GateDecision,
    pub reason: String,
    pub latency_ms: u128,
    pub fallback: bool,
}

impl GateOutcome {
    pub fn fallback(reason: impl Into<String>, latency_ms: u128) -> Self {
        Self {
            decision: GateDecision::Respond,
            reason: reason.into(),
            latency_ms,
            fallback: true,
        }
    }
}

#[derive(Deserialize)]
struct GateArguments {
    decision: String,
    reason: String,
}

pub fn parse_gate_arguments(arguments: &str) -> anyhow::Result<GateOutcome> {
    let arguments: GateArguments =
        serde_json::from_str(arguments).context("Gate arguments were not valid JSON")?;
    let decision = match arguments.decision.as_str() {
        "respond" => GateDecision::Respond,
        "ignore" => GateDecision::Ignore,
        value => anyhow::bail!("unknown Gate decision: {value}"),
    };
    if arguments.reason.trim().is_empty() {
        anyhow::bail!("Gate reason was empty");
    }
    Ok(GateOutcome {
        decision,
        reason: arguments.reason,
        latency_ms: 0,
        fallback: false,
    })
}

pub fn parse_gate_response(
    output: &ResponsesOutput,
    latency_ms: u128,
) -> anyhow::Result<GateOutcome> {
    if output.function_calls.len() != 1 {
        anyhow::bail!(
            "Gate expected one function call, got {}",
            output.function_calls.len()
        );
    }
    let call = &output.function_calls[0];
    if call.name != "decide_response" {
        anyhow::bail!("Gate returned unexpected function: {}", call.name);
    }
    let mut outcome = parse_gate_arguments(&call.arguments)?;
    outcome.latency_ms = latency_ms;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gate_input_contains_recent_dialogue_without_images() {
        let input = build_gate_input(
            &[
                json!({"role":"user","content":"帮我看一下桌面"}),
                json!({"role":"assistant","content":"杯子在左边"}),
            ],
            "再看看右边",
            true,
        );

        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        let text = input[0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("帮我看一下桌面"));
        assert!(text.contains("杯子在左边"));
        assert!(text.contains("再看看右边"));
        assert!(text.contains("assistant_just_replied: true"));
        assert!(!input[0].to_string().contains("input_image"));
    }

    #[test]
    fn parses_gate_function_arguments() {
        let outcome = parse_gate_arguments(
            "{\"decision\":\"ignore\",\"reason\":\"background_conversation\"}",
        )
        .unwrap();

        assert_eq!(outcome.decision, GateDecision::Ignore);
        assert_eq!(outcome.reason, "background_conversation");
        assert!(!outcome.fallback);
    }

    #[test]
    fn rejects_unknown_gate_decision() {
        assert!(parse_gate_arguments("{\"decision\":\"maybe\",\"reason\":\"unclear\"}").is_err());
    }

    #[test]
    fn gate_error_fails_open() {
        let outcome = GateOutcome::fallback("timeout", 3_000);

        assert_eq!(outcome.decision, GateDecision::Respond);
        assert_eq!(outcome.reason, "timeout");
        assert_eq!(outcome.latency_ms, 3_000);
        assert!(outcome.fallback);
    }

    #[test]
    fn accepts_exactly_one_decide_response_call() {
        let output = crate::adapters::ResponsesOutput {
            text: String::new(),
            function_calls: vec![crate::adapters::FunctionCall {
                call_id: "call_gate".to_owned(),
                name: "decide_response".to_owned(),
                arguments: "{\"decision\":\"respond\",\"reason\":\"direct_question\"}".to_owned(),
            }],
            output_items: Vec::new(),
        };

        let outcome = parse_gate_response(&output, 42).unwrap();

        assert_eq!(outcome.decision, GateDecision::Respond);
        assert_eq!(outcome.latency_ms, 42);
        assert!(!outcome.fallback);
    }
}
