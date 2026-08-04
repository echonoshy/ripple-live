use std::{f32::consts::PI, pin::Pin};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{Stream, StreamExt, future};
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::{audio::float32_to_wav, config::Settings, protocol::VideoFrame};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct ResponsesOutput {
    pub text: String,
    pub function_calls: Vec<FunctionCall>,
    pub output_items: Vec<Value>,
}

pub type AudioStream = Pin<Box<dyn Stream<Item = anyhow::Result<Vec<f32>>> + Send>>;
pub type AgentStream = Pin<Box<dyn Stream<Item = anyhow::Result<Value>> + Send>>;

fn agent_rejection_summary(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .pointer("/error/message")
                .and_then(Value::as_str)
                .and_then(|message| message.lines().next())
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(|message| message.chars().take(160).collect())
        })
        .unwrap_or_else(|| "upstream rejected request".to_owned())
}

pub fn reject_legacy_tool_markup(text: &str) -> anyhow::Result<()> {
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("<tool_call") || normalized.contains("</tool_call") {
        anyhow::bail!("legacy tool tag appeared in assistant text")
    }
    Ok(())
}

async fn require_agent_success(response: reqwest::Response) -> anyhow::Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status.is_client_error() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "AGENT_REQUEST_REJECTED status={} summary={}",
            status.as_u16(),
            agent_rejection_summary(&body)
        );
    }
    Ok(response.error_for_status()?)
}

pub fn parse_responses_output(payload: &Value) -> anyhow::Result<ResponsesOutput> {
    let output_items = payload
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .context("Responses payload did not include output[]")?;
    let mut text = String::new();
    let mut function_calls = Vec::new();
    for item in &output_items {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if part.get("type").and_then(Value::as_str) == Some("output_text")
                            && let Some(delta) = part.get("text").and_then(Value::as_str)
                        {
                            text.push_str(delta);
                        }
                    }
                }
            }
            Some("function_call") => {
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .context("function_call did not include arguments")?;
                serde_json::from_str::<Value>(arguments)
                    .context("function_call arguments were not valid JSON")?;
                function_calls.push(FunctionCall {
                    call_id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .context("function_call did not include call_id")?
                        .to_owned(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .context("function_call did not include name")?
                        .to_owned(),
                    arguments: arguments.to_owned(),
                });
            }
            _ => {}
        }
    }
    reject_legacy_tool_markup(&text)?;
    Ok(ResponsesOutput {
        text,
        function_calls,
        output_items,
    })
}

pub fn function_call_output(call_id: &str, result: &Value) -> Value {
    json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": serde_json::to_string(result).unwrap_or_else(|_| "{}".to_owned())
    })
}

pub fn responses_tool_schema(schema: &Value) -> anyhow::Result<Value> {
    let function = schema
        .get("function")
        .and_then(Value::as_object)
        .context("tool schema did not include function")?;
    Ok(json!({
        "type": "function",
        "name": function
            .get("name")
            .and_then(Value::as_str)
            .context("tool schema did not include function.name")?,
        "description": function
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "parameters": function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object","properties":{}}))
    }))
}

pub fn build_responses_user_input(transcript: &str, frames: &[VideoFrame]) -> Value {
    let mut text = transcript.to_owned();
    if !frames.is_empty() {
        text.push_str(&format!(
            "\n\n系统附带了 {} 张按时间先后排列的近期摄像头画面，请只在与用户问题相关时使用画面信息。",
            frames.len()
        ));
    }
    let mut content = vec![json!({"type": "input_text", "text": text})];
    for frame in frames {
        content.push(json!({
            "type": "input_image",
            "detail": "auto",
            "image_url": format!(
                "data:{};base64,{}",
                frame.mime_type,
                STANDARD.encode(&frame.bytes)
            )
        }));
    }
    json!({"role": "user", "content": content})
}

fn responses_request_body(
    model: &str,
    input: &[Value],
    tools: &[Value],
    tool_choice: Value,
    instructions: &str,
    temperature: f64,
    max_output_tokens: u32,
    stream: bool,
) -> Value {
    json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "tools": tools,
        "tool_choice": tool_choice,
        "temperature": temperature,
        "max_output_tokens": max_output_tokens,
        "stream": stream
    })
}

#[derive(Clone)]
pub struct ModelAdapters {
    settings: Settings,
    client: reqwest::Client,
}

impl ModelAdapters {
    pub fn new(settings: Settings) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(settings.request_timeout)
            .pool_max_idle_per_host(32)
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self { settings, client })
    }

    pub async fn transcribe(&self, samples: &[f32]) -> anyhow::Result<String> {
        if self.settings.asr_backend == "mock" {
            return Ok("这是本地模拟语音输入。".to_owned());
        }
        let wav = float32_to_wav(samples, self.settings.sample_rate_in)?;
        let form = Form::new()
            .text("model", self.settings.asr_model.clone())
            .part(
                "file",
                Part::bytes(wav)
                    .file_name("speech.wav")
                    .mime_str("audio/wav")?,
            );
        let response = self
            .client
            .post(&self.settings.asr_url)
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;
        let payload: Value = response.json().await?;
        Ok(normalize_asr_text(
            payload.get("text").and_then(Value::as_str).unwrap_or(""),
        ))
    }

    pub async fn respond(
        &self,
        input: &[Value],
        tools: &[Value],
        tool_choice: Value,
        instructions: &str,
    ) -> anyhow::Result<ResponsesOutput> {
        if self.settings.agent_backend == "mock" {
            return parse_responses_output(&json!({"output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "本地 Agent 网关已经收到你的输入。"}]
            }]}));
        }
        let response = self
            .client
            .post(&self.settings.agent_url)
            .bearer_auth(&self.settings.agent_api_key)
            .json(&responses_request_body(
                &self.settings.agent_model,
                input,
                tools,
                tool_choice,
                instructions,
                self.settings.agent_temperature,
                self.settings.agent_max_tokens,
                false,
            ))
            .send()
            .await?;
        let response = require_agent_success(response).await?;
        parse_responses_output(&response.json::<Value>().await?)
    }

    pub async fn respond_stream(
        &self,
        input: &[Value],
        tools: &[Value],
        tool_choice: Value,
        instructions: &str,
    ) -> anyhow::Result<AgentStream> {
        if self.settings.agent_backend == "mock" {
            return Ok(Box::pin(futures_util::stream::iter([
                Ok(
                    json!({"type":"response.output_text.delta","delta":"本地 Agent 网关已经收到你的输入。"}),
                ),
                Ok(json!({"type":"response.completed","response":{"status":"completed"}})),
            ])));
        }
        let response = self
            .client
            .post(&self.settings.agent_url)
            .bearer_auth(&self.settings.agent_api_key)
            .json(&responses_request_body(
                &self.settings.agent_model,
                input,
                tools,
                tool_choice,
                instructions,
                self.settings.agent_temperature,
                self.settings.agent_max_tokens,
                true,
            ))
            .send()
            .await?;
        let response = require_agent_success(response).await?;
        let mut upstream = response.bytes_stream();
        let stream = async_stream::try_stream! {
            let mut buffer = Vec::<u8>::new();
            while let Some(chunk) = upstream.next().await {
                buffer.extend_from_slice(&chunk?);
                while let Some(boundary) = find_sse_boundary(&buffer) {
                    let event = buffer.drain(..boundary).collect::<Vec<_>>();
                    buffer.drain(..sse_separator_len(&buffer));
                    let event = String::from_utf8(event)?;
                    for line in event.lines() {
                        let Some(data) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let data = data.trim();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }
                        yield serde_json::from_str::<Value>(data)?;
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }

    pub async fn synthesize_stream(&self, text: &str) -> anyhow::Result<AudioStream> {
        if self.settings.tts_backend == "mock" {
            return Ok(Box::pin(futures_util::stream::iter([Ok(mock_tone(
                text,
                self.settings.sample_rate_out,
            ))])));
        }
        let response = self
            .client
            .post(&self.settings.tts_url)
            .json(&json!({
                "model": self.settings.tts_model,
                "input": text,
                "voice": self.settings.tts_voice,
                "language": self.settings.tts_language,
                "instructions": self.settings.tts_instructions,
                "response_format": "pcm",
                "stream": true,
                "stream_format": "audio"
            }))
            .send()
            .await?
            .error_for_status()?;

        // HTTP chunks may split an i16 sample. Carry one trailing byte forward
        // so transport chunking cannot corrupt the raw PCM stream.
        let stream = response.bytes_stream().scan(None::<u8>, |trailing, item| {
            let converted = match item {
                Ok(bytes) => {
                    let mut pcm = Vec::with_capacity(bytes.len() + usize::from(trailing.is_some()));
                    if let Some(byte) = trailing.take() {
                        pcm.push(byte);
                    }
                    pcm.extend_from_slice(&bytes);
                    if !pcm.len().is_multiple_of(2) {
                        *trailing = pcm.pop();
                    }
                    Ok(pcm
                        .chunks_exact(2)
                        .map(|sample| {
                            f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0
                        })
                        .collect())
                }
                Err(error) => Err(anyhow::Error::from(error)),
            };
            future::ready(Some(converted))
        });
        Ok(Box::pin(stream))
    }
}

fn find_sse_boundary(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .or_else(|| buffer.windows(4).position(|window| window == b"\r\n\r\n"))
}

fn sse_separator_len(buffer: &[u8]) -> usize {
    if buffer.starts_with(b"\r\n\r\n") {
        4
    } else {
        2
    }
}

pub fn normalize_asr_text(text: &str) -> String {
    let after_marker = text
        .split_once("<asr_text>")
        .map(|(_, value)| value)
        .unwrap_or(text);
    let without_tags = after_marker
        .replace("<asr_text>", "")
        .replace("</asr_text>", "");
    let trimmed = without_tags.trim();
    if trimmed.to_ascii_lowercase().starts_with("language ") {
        trimmed
            .split_once(char::is_whitespace)
            .and_then(|(_, rest)| rest.split_once(char::is_whitespace))
            .map(|(_, rest)| rest.trim().to_owned())
            .unwrap_or_default()
    } else {
        trimmed.to_owned()
    }
}

fn mock_tone(text: &str, sample_rate: u32) -> Vec<f32> {
    let duration = (text.chars().count() as f32 * 0.025).clamp(0.25, 1.5);
    let length = (duration * sample_rate as f32).round() as usize;
    (0..length)
        .map(|index| {
            let position = index as f32;
            let attack = (position / (sample_rate as f32 * 0.02)).min(1.0);
            let release = ((length - index) as f32 / (sample_rate as f32 * 0.03)).min(1.0);
            0.06 * (2.0 * PI * 440.0 * position / sample_rate as f32).sin() * attack * release
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_agent_rejection_without_echoing_private_input() {
        let body = r#"{"error":{"message":"249 validation errors:\nprivate image payload"}}"#;

        assert_eq!(agent_rejection_summary(body), "249 validation errors:");
        assert!(!agent_rejection_summary(body).contains("private image payload"));
    }

    #[test]
    fn parses_completed_responses_output_items() {
        let output = parse_responses_output(&json!({"output": [
            {"type":"message","content":[{"type":"output_text","text":"结果"}]},
            {"type":"function_call","call_id":"call_7","name":"calculate","arguments":"{\"expression\":\"7*8\"}"}
        ]}))
        .unwrap();

        assert_eq!(output.text, "结果");
        assert_eq!(output.function_calls.len(), 1);
        assert_eq!(output.function_calls[0].call_id, "call_7");
        assert_eq!(output.function_calls[0].name, "calculate");
        assert_eq!(
            output.function_calls[0].arguments,
            "{\"expression\":\"7*8\"}"
        );
    }

    #[test]
    fn rejects_legacy_tool_tag_in_message_text() {
        let error = parse_responses_output(&json!({"output": [{
            "type":"message",
            "content":[{"type":"output_text","text":"<tool_call>{}</tool_call>"}]
        }]}))
        .unwrap_err();

        assert!(error.to_string().contains("legacy tool tag"));
    }

    #[test]
    fn rejects_tagged_function_call_arguments() {
        let error = parse_responses_output(&json!({"output": [{
            "type":"function_call",
            "call_id":"call_bad",
            "name":"calculate",
            "arguments":"<tool_call>{\"name\":\"calculate\"}</tool_call>"
        }]}))
        .unwrap_err();

        assert!(error.to_string().contains("valid JSON"));
    }

    #[test]
    fn builds_responses_function_call_output_item() {
        assert_eq!(
            function_call_output("call_7", &json!({"ok":true,"result":56})),
            json!({
                "type":"function_call_output",
                "call_id":"call_7",
                "output":"{\"ok\":true,\"result\":56}"
            })
        );
    }

    #[test]
    fn converts_chat_tool_schema_to_responses_shape() {
        let converted = responses_tool_schema(&json!({
            "type":"function",
            "function":{
                "name":"calculate",
                "description":"计算表达式",
                "parameters":{"type":"object"}
            }
        }))
        .unwrap();

        assert_eq!(
            converted,
            json!({
                "type":"function",
                "name":"calculate",
                "description":"计算表达式",
                "parameters":{"type":"object"}
            })
        );
    }

    #[test]
    fn builds_responses_multimodal_user_input() {
        let frames = vec![VideoFrame {
            bytes: vec![1, 2, 3],
            mime_type: "image/jpeg".to_owned(),
            captured_at_ms: Some(1),
            received_at_ms: 2,
        }];

        let input = build_responses_user_input("看看这里", &frames);

        assert_eq!(input["role"], "user");
        assert_eq!(input["content"][0]["type"], "input_text");
        assert_eq!(input["content"][1]["type"], "input_image");
        assert_eq!(input["content"][1]["detail"], "auto");
        assert_eq!(
            input["content"][1]["image_url"],
            "data:image/jpeg;base64,AQID"
        );
    }

    #[test]
    fn builds_native_responses_request_body() {
        let body = responses_request_body(
            "Qwen3-VL-8B-Instruct",
            &[json!({"role":"user","content":[{"type":"input_text","text":"你好"}]})],
            &[json!({"type":"function","name":"calculate","parameters":{"type":"object"}})],
            json!("auto"),
            "system instructions",
            0.2,
            256,
            true,
        );

        assert_eq!(body["model"], "Qwen3-VL-8B-Instruct");
        assert!(body.get("input").is_some());
        assert!(body.get("messages").is_none());
        assert_eq!(body["max_output_tokens"], 256);
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn strips_asr_metadata() {
        assert_eq!(
            normalize_asr_text("language Chinese<asr_text>你好</asr_text>"),
            "你好"
        );
    }
}
