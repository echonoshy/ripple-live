use std::{f32::consts::PI, pin::Pin};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{Stream, StreamExt, future};
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::{audio::float32_to_wav, config::Settings, protocol::VideoFrame};

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct AgentReply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub raw_message: Value,
}

pub type AudioStream = Pin<Box<dyn Stream<Item = anyhow::Result<Vec<f32>>> + Send>>;
pub type AgentStream = Pin<Box<dyn Stream<Item = anyhow::Result<Value>> + Send>>;

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

    pub async fn complete(
        &self,
        messages: &[Value],
        tools: &[Value],
        tool_choice: Value,
    ) -> anyhow::Result<AgentReply> {
        self.complete_with_options(
            messages,
            tools,
            tool_choice,
            self.settings.agent_temperature,
            self.settings.agent_max_tokens,
        )
        .await
    }

    pub async fn classify_turn_end(&self, transcript: &str) -> anyhow::Result<String> {
        self.complete_with_options(
            &[
                json!({"role":"system","content":"Return only JSON: {\\\"decision\\\":\\\"complete|continue\\\",\\\"confidence\\\":0..1}. Mark complete only when the Chinese utterance is clearly finished."}),
                json!({"role":"user","content":transcript}),
            ],
            &[],
            json!("none"),
            0.0,
            64,
        )
        .await
        .map(|reply| reply.content)
    }

    async fn complete_with_options(
        &self,
        messages: &[Value],
        tools: &[Value],
        tool_choice: Value,
        temperature: f64,
        max_tokens: u32,
    ) -> anyhow::Result<AgentReply> {
        if self.settings.agent_backend == "mock" {
            let content = "本地 Agent 网关已经收到你的输入。".to_owned();
            return Ok(AgentReply {
                content: content.clone(),
                tool_calls: Vec::new(),
                raw_message: json!({"role": "assistant", "content": content}),
            });
        }
        let response = self
            .client
            .post(&self.settings.agent_url)
            .bearer_auth(&self.settings.agent_api_key)
            .json(&json!({
                "model": self.settings.agent_model,
                "messages": messages,
                "tools": tools,
                "tool_choice": tool_choice,
                "temperature": temperature,
                "max_tokens": max_tokens
            }))
            .send()
            .await?
            .error_for_status()?;
        let payload: Value = response.json().await?;
        let message = payload
            .pointer("/choices/0/message")
            .cloned()
            .context("agent response did not include choices[0].message")?;
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .map(|call| ToolCall {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        arguments: call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(AgentReply {
            content: message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            tool_calls,
            raw_message: message,
        })
    }

    pub async fn complete_stream(
        &self,
        messages: &[Value],
        tools: &[Value],
        tool_choice: Value,
    ) -> anyhow::Result<AgentStream> {
        if self.settings.agent_backend == "mock" {
            let content = "本地 Agent 网关已经收到你的输入。";
            return Ok(Box::pin(futures_util::stream::iter([
                Ok(json!({"choices": [{"delta": {"role": "assistant", "content": content}}]})),
                Ok(json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})),
            ])));
        }
        let response = self
            .client
            .post(&self.settings.agent_url)
            .bearer_auth(&self.settings.agent_api_key)
            .json(&json!({
                "model": self.settings.agent_model,
                "messages": messages,
                "tools": tools,
                "tool_choice": tool_choice,
                "temperature": self.settings.agent_temperature,
                "max_tokens": self.settings.agent_max_tokens,
                "stream": true
            }))
            .send()
            .await?
            .error_for_status()?;
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

pub fn build_multimodal_user_message(transcript: &str, frames: &[VideoFrame]) -> Value {
    let mut text = transcript.to_owned();
    if !frames.is_empty() {
        text.push_str(&format!(
            "\n\n系统附带了 {} 张按时间先后排列的近期摄像头画面，请只在与用户问题相关时使用画面信息。",
            frames.len()
        ));
    }
    let mut content = vec![json!({"type": "text", "text": text})];
    for frame in frames {
        content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!(
                    "data:{};base64,{}",
                    frame.mime_type,
                    STANDARD.encode(&frame.bytes)
                )
            }
        }));
    }
    json!({"role": "user", "content": content})
}

pub fn tool_result_message(call: &ToolCall, result: &Value) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": call.id,
        "name": call.name,
        "content": serde_json::to_string(result).unwrap_or_else(|_| "{}".to_owned())
    })
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
    fn strips_asr_metadata() {
        assert_eq!(
            normalize_asr_text("language Chinese<asr_text>你好</asr_text>"),
            "你好"
        );
    }
}
