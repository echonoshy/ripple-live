use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::{adapters::ModelAdapters, context::ContextStore};

#[derive(Clone)]
pub struct MeetingService {
    context: ContextStore,
    adapters: ModelAdapters,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeetingRecord {
    pub id: String,
    pub conversation_id: Option<String>,
    pub mode: String,
    pub status: String,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    pub duration_seconds: Option<i64>,
    pub title: String,
    pub summary: String,
    pub generated_at: Option<f64>,
    pub last_error: Option<String>,
    pub transcript_count: i64,
    pub action_count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TranscriptSegment {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: f64,
    pub ordinal: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeetingActionItem {
    pub id: String,
    pub title: String,
    pub due_at: Option<f64>,
    pub todo_id: Option<String>,
    pub ordinal: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeetingDetail {
    #[serde(flatten)]
    pub meeting: MeetingRecord,
    pub transcript: Vec<TranscriptSegment>,
    pub action_items: Vec<MeetingActionItem>,
}

#[derive(Debug, Deserialize)]
struct GeneratedNotes {
    title: String,
    summary: String,
    #[serde(default)]
    action_items: Vec<GeneratedActionItem>,
}

#[derive(Debug, Deserialize)]
struct GeneratedActionItem {
    title: String,
}

impl MeetingService {
    pub fn new(context: ContextStore, adapters: ModelAdapters) -> Self {
        Self { context, adapters }
    }

    pub async fn start(
        &self,
        user_id: &str,
        conversation_id: &str,
        mode: &str,
    ) -> anyhow::Result<MeetingRecord> {
        if mode != "audio" && mode != "video" {
            anyhow::bail!("会议模式只支持 audio 或 video");
        }
        if !self
            .context
            .conversation_belongs_to(conversation_id, user_id)
            .await?
        {
            anyhow::bail!("对话不存在");
        }
        let id = format!("meet_{}", Uuid::new_v4().simple());
        let now = unix_time();
        sqlx::query(
            "INSERT INTO meetings(
                id, user_id, conversation_id, mode, status, started_at,
                created_at, updated_at
             ) VALUES ($1, $2, $3, $4, 'recording', $5, $6, $7)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(conversation_id)
        .bind(mode)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(self.context.pool())
        .await?;
        self.get_record(user_id, &id)
            .await?
            .context("创建后的会议记录不存在")
    }

    pub async fn list(&self, user_id: &str, limit: i64) -> anyhow::Result<Vec<MeetingRecord>> {
        let rows = sqlx::query(
            "SELECT m.*,
                (SELECT COUNT(*) FROM meeting_transcript_segments s WHERE s.meeting_id = m.id) AS transcript_count,
                (SELECT COUNT(*) FROM meeting_action_items a WHERE a.meeting_id = m.id) AS action_count
             FROM meetings m WHERE m.user_id = $1
             ORDER BY m.started_at DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(self.context.pool())
        .await?;
        Ok(rows.iter().map(meeting_from_row).collect())
    }

    pub async fn get(&self, user_id: &str, id: &str) -> anyhow::Result<Option<MeetingDetail>> {
        let Some(meeting) = self.get_record(user_id, id).await? else {
            return Ok(None);
        };
        let transcript = sqlx::query(
            "SELECT id, role, content, created_at, ordinal
             FROM meeting_transcript_segments WHERE meeting_id = $1
             ORDER BY ordinal ASC",
        )
        .bind(id)
        .fetch_all(self.context.pool())
        .await?
        .into_iter()
        .map(|row| TranscriptSegment {
            id: row.get("id"),
            role: row.get("role"),
            content: row.get("content"),
            created_at: row.get("created_at"),
            ordinal: row.get("ordinal"),
        })
        .collect();
        let action_items = self.action_items(id).await?;
        Ok(Some(MeetingDetail {
            meeting,
            transcript,
            action_items,
        }))
    }

    pub async fn finish(&self, user_id: &str, id: &str) -> anyhow::Result<Option<MeetingRecord>> {
        let Some(current) = self.get_record(user_id, id).await? else {
            return Ok(None);
        };
        if current.status == "ready" || current.status == "processing" {
            return Ok(Some(current));
        }
        let conversation_id = current
            .conversation_id
            .as_deref()
            .context("会议关联的对话已经不存在")?;
        let ended_at = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        sqlx::query("DELETE FROM meeting_transcript_segments WHERE meeting_id = $1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO meeting_transcript_segments(meeting_id, role, content, created_at, ordinal)
             SELECT $1, role, content, created_at,
                    ROW_NUMBER() OVER (ORDER BY id ASC) - 1
             FROM turns
             WHERE session_id = $2 AND created_at >= $3 AND created_at <= $4
               AND role IN ('user', 'assistant') AND LENGTH(TRIM(content)) > 0
             ORDER BY id ASC",
        )
        .bind(id)
        .bind(conversation_id)
        .bind(current.started_at)
        .bind(ended_at)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE meetings SET status = 'processing', ended_at = $1,
                duration_seconds = GREATEST(0, ROUND($1 - started_at)::BIGINT),
                last_error = NULL, updated_at = $1
             WHERE id = $2 AND user_id = $3",
        )
        .bind(ended_at)
        .bind(id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_record(user_id, id).await
    }

    pub async fn mark_processing(
        &self,
        user_id: &str,
        id: &str,
    ) -> anyhow::Result<Option<MeetingRecord>> {
        let result = sqlx::query(
            "UPDATE meetings SET status = 'processing', last_error = NULL, updated_at = $1
             WHERE id = $2 AND user_id = $3 AND ended_at IS NOT NULL",
        )
        .bind(unix_time())
        .bind(id)
        .bind(user_id)
        .execute(self.context.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_record(user_id, id).await
    }

    pub async fn generate(&self, user_id: &str, id: &str) -> anyhow::Result<()> {
        let detail = self.get(user_id, id).await?.context("会议记录不存在")?;
        let transcript = detail
            .transcript
            .iter()
            .map(|segment| {
                let speaker = if segment.role == "user" {
                    "用户"
                } else {
                    "Ripple"
                };
                format!("{speaker}: {}", segment.content.trim())
            })
            .collect::<Vec<_>>()
            .join("\n");
        let generated = if transcript.trim().is_empty() {
            GeneratedNotes {
                title: "未命名会议".to_owned(),
                summary: "本次会议没有识别到可用的逐字稿。".to_owned(),
                action_items: Vec::new(),
            }
        } else {
            self.generate_notes(&transcript).await?
        };
        let title = bounded_text(&generated.title, 80, "未命名会议");
        let summary = bounded_text(&generated.summary, 4_000, "暂无摘要");
        let now = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        sqlx::query("DELETE FROM meeting_action_items WHERE meeting_id = $1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        for (ordinal, action) in generated.action_items.into_iter().take(10).enumerate() {
            let action_title = bounded_text(&action.title, 500, "");
            if action_title.is_empty() {
                continue;
            }
            sqlx::query(
                "INSERT INTO meeting_action_items(
                    id, meeting_id, title, due_at, todo_id, ordinal, created_at
                 ) VALUES ($1, $2, $3, NULL, NULL, $4, $5)",
            )
            .bind(format!("ma_{}", Uuid::new_v4().simple()))
            .bind(id)
            .bind(action_title)
            .bind(ordinal as i64)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE meetings SET status = 'ready', title = $1, summary = $2,
                generated_at = $3, last_error = NULL, updated_at = $3
             WHERE id = $4 AND user_id = $5",
        )
        .bind(title)
        .bind(summary)
        .bind(now)
        .bind(id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn mark_failed(&self, user_id: &str, id: &str, error: &str) {
        let message: String = error.chars().take(500).collect();
        let _ = sqlx::query(
            "UPDATE meetings SET status = 'failed', last_error = $1, updated_at = $2
             WHERE id = $3 AND user_id = $4",
        )
        .bind(message)
        .bind(unix_time())
        .bind(id)
        .bind(user_id)
        .execute(self.context.pool())
        .await;
    }

    pub async fn promote_action(
        &self,
        user_id: &str,
        meeting_id: &str,
        action_id: &str,
    ) -> anyhow::Result<Option<MeetingActionItem>> {
        let mut transaction = self.context.pool().begin().await?;
        let row = sqlx::query(
            "SELECT a.title, a.due_at, a.todo_id, a.ordinal, m.conversation_id
             FROM meeting_action_items a
             JOIN meetings m ON m.id = a.meeting_id
             WHERE a.id = $1 AND a.meeting_id = $2 AND m.user_id = $3
             FOR UPDATE OF a",
        )
        .bind(action_id)
        .bind(meeting_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.get::<Option<String>, _>("todo_id").is_none() {
            let todo_id = format!("todo_{}", Uuid::new_v4().simple());
            let now = unix_time();
            sqlx::query(
                "INSERT INTO todos(
                    id, user_id, conversation_id, meeting_id, title, visual_summary,
                    due_at, completed_at, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, '', $6, NULL, $7, $8)",
            )
            .bind(&todo_id)
            .bind(user_id)
            .bind(row.get::<Option<String>, _>("conversation_id"))
            .bind(meeting_id)
            .bind(row.get::<String, _>("title"))
            .bind(row.get::<Option<f64>, _>("due_at"))
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE meeting_action_items SET todo_id = $1 WHERE id = $2 AND todo_id IS NULL",
            )
            .bind(todo_id)
            .bind(action_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(self
            .action_items(meeting_id)
            .await?
            .into_iter()
            .find(|item| item.id == action_id))
    }

    async fn generate_notes(&self, transcript: &str) -> anyhow::Result<GeneratedNotes> {
        let tool = json!({
            "type": "function",
            "name": "save_meeting_notes",
            "description": "保存会议标题、摘要和明确行动项",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "title": {"type": "string"},
                    "summary": {"type": "string"},
                    "action_items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {"title": {"type": "string"}},
                            "required": ["title"]
                        }
                    }
                },
                "required": ["title", "summary", "action_items"]
            }
        });
        let input = vec![json!({
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": format!("请整理以下会议逐字稿：\n\n{transcript}")
            }]
        })];
        let output = self
            .adapters
            .respond(
                &input,
                &[tool],
                json!({"type": "function", "name": "save_meeting_notes"}),
                "你是会议记录整理助手。标题使用简洁中文；摘要忠于原文；只有逐字稿中存在明确行动要求时才生成行动项，不推测负责人、截止日期或未说过的事实。",
            )
            .await?;
        let call = output
            .function_calls
            .into_iter()
            .find(|call| call.name == "save_meeting_notes")
            .context("会议整理模型没有返回结构化结果")?;
        serde_json::from_str(&call.arguments).context("会议整理结果格式不正确")
    }

    async fn get_record(&self, user_id: &str, id: &str) -> anyhow::Result<Option<MeetingRecord>> {
        let row = sqlx::query(
            "SELECT m.*,
                (SELECT COUNT(*) FROM meeting_transcript_segments s WHERE s.meeting_id = m.id) AS transcript_count,
                (SELECT COUNT(*) FROM meeting_action_items a WHERE a.meeting_id = m.id) AS action_count
             FROM meetings m WHERE m.id = $1 AND m.user_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        Ok(row.as_ref().map(meeting_from_row))
    }

    async fn action_items(&self, meeting_id: &str) -> anyhow::Result<Vec<MeetingActionItem>> {
        Ok(sqlx::query(
            "SELECT id, title, due_at, todo_id, ordinal
             FROM meeting_action_items WHERE meeting_id = $1 ORDER BY ordinal ASC",
        )
        .bind(meeting_id)
        .fetch_all(self.context.pool())
        .await?
        .into_iter()
        .map(|row| MeetingActionItem {
            id: row.get("id"),
            title: row.get("title"),
            due_at: row.get("due_at"),
            todo_id: row.get("todo_id"),
            ordinal: row.get("ordinal"),
        })
        .collect())
    }
}

fn meeting_from_row(row: &sqlx::postgres::PgRow) -> MeetingRecord {
    MeetingRecord {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        mode: row.get("mode"),
        status: row.get("status"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        duration_seconds: row.get("duration_seconds"),
        title: row.get("title"),
        summary: row.get("summary"),
        generated_at: row.get("generated_at"),
        last_error: row.get("last_error"),
        transcript_count: row.get("transcript_count"),
        action_count: row.get("action_count"),
    }
}

fn bounded_text(value: &str, max_chars: usize, fallback: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() { fallback } else { value };
    value.chars().take(max_chars).collect()
}

fn unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::bounded_text;

    #[test]
    fn meeting_text_is_trimmed_bounded_and_has_a_fallback() {
        assert_eq!(bounded_text("  会议标题  ", 8, "默认"), "会议标题");
        assert_eq!(bounded_text("一二三四五", 3, "默认"), "一二三");
        assert_eq!(bounded_text("   ", 8, "默认"), "默认");
    }
}
