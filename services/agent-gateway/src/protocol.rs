use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    Audio,
    Video,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionPurpose {
    #[default]
    Conversation,
    Meeting,
}

impl SessionPurpose {
    pub fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.unwrap_or("conversation") {
            "conversation" => Ok(Self::Conversation),
            "meeting" => Ok(Self::Meeting),
            other => anyhow::bail!("不支持的会话用途: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Meeting => "meeting",
        }
    }
}

impl SessionMode {
    pub fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value {
            Some("audio") => Ok(Self::Audio),
            Some("video") => Ok(Self::Video),
            Some(other) => anyhow::bail!("不支持的会话模式: {other}"),
            None => anyhow::bail!("会话模式不能为空"),
        }
    }

    pub fn parse_initial(value: Option<&str>) -> anyhow::Result<Self> {
        Self::parse(Some(value.unwrap_or("audio")))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ClientEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub audio: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub response_id: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub captured_at_ms: Option<i64>,
    pub received_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::{SessionMode, SessionPurpose};

    #[test]
    fn session_mode_accepts_only_audio_and_video() {
        assert_eq!(
            SessionMode::parse(Some("audio")).unwrap(),
            SessionMode::Audio
        );
        assert_eq!(
            SessionMode::parse(Some("video")).unwrap(),
            SessionMode::Video
        );
        assert_eq!(
            SessionMode::parse_initial(None).unwrap(),
            SessionMode::Audio
        );
        assert!(SessionMode::parse(None).is_err());
        assert!(SessionMode::parse(Some("continuous_video")).is_err());
    }

    #[test]
    fn session_purpose_defaults_to_conversation_and_accepts_meeting() {
        assert_eq!(
            SessionPurpose::parse(None).unwrap(),
            SessionPurpose::Conversation
        );
        assert_eq!(
            SessionPurpose::parse(Some("meeting")).unwrap(),
            SessionPurpose::Meeting
        );
        assert!(SessionPurpose::parse(Some("assistant")).is_err());
    }
}
