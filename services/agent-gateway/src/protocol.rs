use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct ClientEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub audio: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}
