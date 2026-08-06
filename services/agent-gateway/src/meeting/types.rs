use anyhow::bail;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingState {
    Recording,
    Paused,
    Uploading,
    Processing,
    Completed,
    Interrupted,
}

impl MeetingState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Paused => "paused",
            Self::Uploading => "uploading",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "recording" => Ok(Self::Recording),
            "paused" => Ok(Self::Paused),
            "uploading" => Ok(Self::Uploading),
            "processing" => Ok(Self::Processing),
            "completed" => Ok(Self::Completed),
            "interrupted" => Ok(Self::Interrupted),
            _ => bail!("unknown meeting state: {value}"),
        }
    }

    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (
                    Self::Recording,
                    Self::Paused | Self::Uploading | Self::Interrupted
                ) | (
                    Self::Paused,
                    Self::Recording | Self::Uploading | Self::Interrupted
                ) | (Self::Interrupted, Self::Uploading)
                    | (Self::Uploading, Self::Processing)
                    | (Self::Processing, Self::Completed)
            )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    Upload,
    Transcript,
    Organization,
}

impl ProcessingStage {
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "upload" => Ok(Self::Upload),
            "transcript" => Ok(Self::Transcript),
            "organization" => Ok(Self::Organization),
            _ => bail!("unknown meeting processing stage: {value}"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TranscriptSegment {
    pub id: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub provisional: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MeetingTodo {
    pub id: String,
    pub text: String,
    pub completed: bool,
    pub source_start_ms: Option<i64>,
    pub source_end_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Meeting {
    pub id: String,
    pub state: MeetingState,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub duration_ms: Option<i64>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub error_stage: Option<ProcessingStage>,
    pub error_message: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub transcript: Vec<TranscriptSegment>,
    pub todos: Vec<MeetingTodo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkWrite {
    Inserted,
    Existing,
    Conflict,
}
