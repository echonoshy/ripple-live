use std::{
    collections::HashSet,
    path::{Component, Path},
    time::SystemTime,
};

use anyhow::{Context, bail};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::types::{
    ChunkWrite, FinalAudioMetadata, Meeting, MeetingState, MeetingTodo, ProcessingStage,
    StoredChunkMetadata, TranscriptSegment,
};

#[derive(Clone)]
pub struct MeetingStore {
    pool: SqlitePool,
}

impl MeetingStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        for statement in [
            "CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN (
                    'recording', 'paused', 'uploading', 'processing', 'completed', 'interrupted'
                )),
                started_at REAL NOT NULL,
                ended_at REAL,
                duration_ms INTEGER,
                title TEXT,
                summary TEXT,
                final_audio_path TEXT,
                final_audio_size_bytes INTEGER CHECK(final_audio_size_bytes IS NULL OR final_audio_size_bytes >= 0),
                final_audio_checksum TEXT,
                error_stage TEXT CHECK(error_stage IS NULL OR error_stage IN (
                    'upload', 'transcript', 'organization'
                )),
                error_message TEXT,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL,
                UNIQUE(user_id, idempotency_key)
            )",
            "CREATE TABLE IF NOT EXISTS meeting_chunks (
                meeting_id TEXT NOT NULL,
                sequence INTEGER NOT NULL CHECK(sequence >= 0),
                start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
                end_ms INTEGER NOT NULL CHECK(end_ms > start_ms),
                checksum TEXT NOT NULL,
                size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
                content_path TEXT,
                verified INTEGER NOT NULL DEFAULT 0,
                created_at REAL NOT NULL,
                PRIMARY KEY(meeting_id, sequence),
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            )",
            "CREATE TABLE IF NOT EXISTS meeting_transcript_segments (
                meeting_id TEXT NOT NULL,
                id INTEGER NOT NULL,
                start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
                end_ms INTEGER NOT NULL CHECK(end_ms > start_ms),
                text TEXT NOT NULL,
                provisional INTEGER NOT NULL CHECK(provisional IN (0, 1)),
                PRIMARY KEY(meeting_id, id),
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            )",
            "CREATE TABLE IF NOT EXISTS meeting_todos (
                meeting_id TEXT NOT NULL,
                id TEXT NOT NULL,
                text TEXT NOT NULL,
                completed INTEGER NOT NULL CHECK(completed IN (0, 1)),
                source_start_ms INTEGER,
                source_end_ms INTEGER,
                created_at REAL NOT NULL,
                updated_at REAL NOT NULL,
                PRIMARY KEY(meeting_id, id),
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
                CHECK((source_start_ms IS NULL AND source_end_ms IS NULL) OR
                      (source_start_ms >= 0 AND source_end_ms > source_start_ms))
            )",
            "CREATE TABLE IF NOT EXISTS meeting_processing_jobs (
                meeting_id TEXT NOT NULL,
                stage TEXT NOT NULL CHECK(stage IN ('upload', 'transcript', 'organization')),
                attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt >= 0),
                status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'running', 'completed', 'failed')),
                diagnostic_error TEXT,
                updated_at REAL NOT NULL,
                PRIMARY KEY(meeting_id, stage),
                FOREIGN KEY(meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            )",
            "CREATE INDEX IF NOT EXISTS idx_meetings_user_updated
             ON meetings(user_id, updated_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_meeting_chunks_timeline
             ON meeting_chunks(meeting_id, start_ms)",
            "CREATE INDEX IF NOT EXISTS idx_meeting_transcript_timeline
             ON meeting_transcript_segments(meeting_id, start_ms)",
            "CREATE INDEX IF NOT EXISTS idx_meeting_todos_meeting
             ON meeting_todos(meeting_id, completed)",
            "CREATE INDEX IF NOT EXISTS idx_meeting_jobs_status
             ON meeting_processing_jobs(status, updated_at)",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        ensure_column(&self.pool, "meetings", "final_audio_path", "TEXT").await?;
        ensure_column(&self.pool, "meetings", "final_audio_size_bytes", "INTEGER").await?;
        ensure_column(&self.pool, "meetings", "final_audio_checksum", "TEXT").await?;
        Ok(())
    }

    pub async fn create(
        &self,
        user_id: &str,
        idempotency_key: &str,
        started_at: f64,
    ) -> anyhow::Result<Meeting> {
        Ok(self
            .create_with_status(user_id, idempotency_key, started_at)
            .await?
            .0)
    }

    pub(crate) async fn create_with_status(
        &self,
        user_id: &str,
        idempotency_key: &str,
        started_at: f64,
    ) -> anyhow::Result<(Meeting, bool)> {
        if user_id.trim().is_empty() || idempotency_key.trim().is_empty() {
            bail!("user_id and idempotency_key must not be empty");
        }
        if !started_at.is_finite() || started_at < 0.0 {
            bail!("started_at must be a non-negative finite timestamp");
        }
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let insert = sqlx::query(
            "INSERT INTO meetings(
                id, user_id, idempotency_key, state, started_at, created_at, updated_at
             ) VALUES (?, ?, ?, 'recording', ?, ?, ?)
             ON CONFLICT(user_id, idempotency_key) DO NOTHING",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(idempotency_key)
        .bind(started_at)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let id: String =
            sqlx::query("SELECT id FROM meetings WHERE user_id = ? AND idempotency_key = ?")
                .bind(user_id)
                .bind(idempotency_key)
                .fetch_one(&mut *transaction)
                .await?
                .get("id");
        transaction.commit().await?;
        let meeting = self
            .get_owned(user_id, &id)
            .await?
            .context("created meeting was not found")?;
        Ok((meeting, insert.rows_affected() == 1))
    }

    pub async fn list(&self, user_id: &str) -> anyhow::Result<Vec<Meeting>> {
        let rows = sqlx::query(
            "SELECT id, state, started_at, ended_at, duration_ms, title, summary,
                    error_stage, error_message, created_at, updated_at
             FROM meetings WHERE user_id = ? ORDER BY started_at DESC, id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(meeting_from_row).collect()
    }

    pub async fn get_owned(
        &self,
        user_id: &str,
        meeting_id: &str,
    ) -> anyhow::Result<Option<Meeting>> {
        let Some(row) = sqlx::query(
            "SELECT id, state, started_at, ended_at, duration_ms, title, summary,
                    error_stage, error_message, created_at, updated_at
             FROM meetings WHERE user_id = ? AND id = ?",
        )
        .bind(user_id)
        .bind(meeting_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let mut meeting = meeting_from_row(row)?;
        meeting.transcript = self.transcript(meeting_id).await?;
        meeting.todos = self.todos(meeting_id).await?;
        Ok(Some(meeting))
    }

    pub async fn record_chunk(
        &self,
        meeting_id: &str,
        sequence: i64,
        start_ms: i64,
        end_ms: i64,
        checksum: &str,
        size_bytes: i64,
    ) -> anyhow::Result<ChunkWrite> {
        if sequence < 0 || start_ms < 0 || end_ms <= start_ms || size_bytes < 0 {
            bail!("invalid meeting chunk metadata");
        }
        if checksum.trim().is_empty() {
            bail!("chunk checksum must not be empty");
        }
        let result = sqlx::query(
            "INSERT INTO meeting_chunks(
                meeting_id, sequence, start_ms, end_ms, checksum, size_bytes, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id, sequence) DO NOTHING",
        )
        .bind(meeting_id)
        .bind(sequence)
        .bind(start_ms)
        .bind(end_ms)
        .bind(checksum)
        .bind(size_bytes)
        .bind(unix_time())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            return Ok(ChunkWrite::Inserted);
        }

        let row = sqlx::query(
            "SELECT start_ms, end_ms, checksum, size_bytes
             FROM meeting_chunks WHERE meeting_id = ? AND sequence = ?",
        )
        .bind(meeting_id)
        .bind(sequence)
        .fetch_one(&self.pool)
        .await?;
        let identical = row.get::<i64, _>("start_ms") == start_ms
            && row.get::<i64, _>("end_ms") == end_ms
            && row.get::<String, _>("checksum") == checksum
            && row.get::<i64, _>("size_bytes") == size_bytes;
        Ok(if identical {
            ChunkWrite::Existing
        } else {
            ChunkWrite::Conflict
        })
    }

    pub async fn missing_sequences(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<i64>> {
        if last_sequence < 0 {
            bail!("last_sequence must not be negative");
        }
        let present = sqlx::query(
            "SELECT sequence FROM meeting_chunks
             WHERE meeting_id = ? AND sequence <= ? ORDER BY sequence",
        )
        .bind(meeting_id)
        .bind(last_sequence)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| row.get::<i64, _>("sequence"))
        .collect::<HashSet<_>>();
        Ok((0..=last_sequence)
            .filter(|sequence| !present.contains(sequence))
            .collect())
    }

    pub async fn record_verified_chunk(
        &self,
        meeting_id: &str,
        sequence: i64,
        start_ms: i64,
        end_ms: i64,
        checksum: &str,
        size_bytes: i64,
        relative_path: &str,
    ) -> anyhow::Result<ChunkWrite> {
        if sequence < 0 || start_ms < 0 || end_ms <= start_ms || size_bytes < 0 {
            bail!("invalid meeting chunk metadata");
        }
        if checksum.trim().is_empty() || !safe_relative_path(relative_path) {
            bail!("invalid verified meeting chunk metadata");
        }
        if relative_path != format!("{meeting_id}/chunks/{sequence}.m4a") {
            bail!("invalid verified meeting chunk path");
        }
        let result = sqlx::query(
            "INSERT INTO meeting_chunks(
                meeting_id, sequence, start_ms, end_ms, checksum, size_bytes,
                content_path, verified, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)
             ON CONFLICT(meeting_id, sequence) DO NOTHING",
        )
        .bind(meeting_id)
        .bind(sequence)
        .bind(start_ms)
        .bind(end_ms)
        .bind(checksum)
        .bind(size_bytes)
        .bind(relative_path)
        .bind(unix_time())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            return Ok(ChunkWrite::Inserted);
        }

        let row = sqlx::query(
            "SELECT start_ms, end_ms, checksum, size_bytes, content_path, verified
             FROM meeting_chunks WHERE meeting_id = ? AND sequence = ?",
        )
        .bind(meeting_id)
        .bind(sequence)
        .fetch_one(&self.pool)
        .await?;
        let identical = row.get::<i64, _>("start_ms") == start_ms
            && row.get::<i64, _>("end_ms") == end_ms
            && row.get::<String, _>("checksum") == checksum
            && row.get::<i64, _>("size_bytes") == size_bytes;
        if !identical {
            return Ok(ChunkWrite::Conflict);
        }
        let stored_path = row.get::<Option<String>, _>("content_path");
        if stored_path
            .as_deref()
            .is_some_and(|stored_path| stored_path != relative_path)
        {
            return Ok(ChunkWrite::Conflict);
        }
        if stored_path.is_none() || !row.get::<bool, _>("verified") {
            sqlx::query(
                "UPDATE meeting_chunks SET content_path = ?, verified = 1
                 WHERE meeting_id = ? AND sequence = ?",
            )
            .bind(relative_path)
            .bind(meeting_id)
            .bind(sequence)
            .execute(&self.pool)
            .await?;
        }
        Ok(ChunkWrite::Existing)
    }

    pub async fn missing_verified_sequences(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<i64>> {
        if last_sequence < 0 {
            bail!("last_sequence must not be negative");
        }
        let present = sqlx::query(
            "SELECT sequence FROM meeting_chunks
             WHERE meeting_id = ? AND sequence <= ?
               AND verified = 1 AND content_path IS NOT NULL
             ORDER BY sequence",
        )
        .bind(meeting_id)
        .bind(last_sequence)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| row.get::<i64, _>("sequence"))
        .collect::<HashSet<_>>();
        Ok((0..=last_sequence)
            .filter(|sequence| !present.contains(sequence))
            .collect())
    }

    pub async fn verified_chunks(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<StoredChunkMetadata>> {
        if last_sequence < 0 {
            bail!("last_sequence must not be negative");
        }
        let rows = sqlx::query(
            "SELECT sequence, content_path, size_bytes, checksum
             FROM meeting_chunks
             WHERE meeting_id = ? AND sequence <= ?
               AND verified = 1 AND content_path IS NOT NULL
             ORDER BY sequence",
        )
        .bind(meeting_id)
        .bind(last_sequence)
        .fetch_all(&self.pool)
        .await?;
        let mut chunks = Vec::with_capacity(rows.len());
        for row in rows {
            let sequence = row.get::<i64, _>("sequence");
            let relative_path = row.get::<String, _>("content_path");
            if !safe_relative_path(&relative_path)
                || relative_path != format!("{meeting_id}/chunks/{sequence}.m4a")
            {
                bail!("unsafe stored meeting chunk path");
            }
            chunks.push(StoredChunkMetadata {
                sequence,
                relative_path,
                size_bytes: row.get("size_bytes"),
                checksum: row.get("checksum"),
            });
        }
        Ok(chunks)
    }

    pub async fn finalize_owned_upload(
        &self,
        user_id: &str,
        meeting_id: &str,
        ended_at: f64,
        audio: &FinalAudioMetadata,
    ) -> anyhow::Result<bool> {
        if !ended_at.is_finite()
            || !safe_relative_path(&audio.relative_path)
            || audio.size_bytes < 0
            || audio.checksum.trim().is_empty()
        {
            bail!("invalid final meeting audio metadata");
        }
        if audio.relative_path != format!("{meeting_id}/recording.m4a") {
            bail!("invalid final meeting audio path");
        }
        let mut transaction = self.pool.begin().await?;
        let Some(row) =
            sqlx::query("SELECT state, started_at FROM meetings WHERE id = ? AND user_id = ?")
                .bind(meeting_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await?
        else {
            transaction.rollback().await?;
            return Ok(false);
        };
        let state = MeetingState::parse(&row.get::<String, _>("state"))?;
        let started_at = row.get::<f64, _>("started_at");
        if ended_at < started_at {
            bail!("ended_at must not precede started_at");
        }
        if !matches!(
            state,
            MeetingState::Recording
                | MeetingState::Paused
                | MeetingState::Interrupted
                | MeetingState::Uploading
                | MeetingState::Processing
        ) {
            bail!("meeting cannot be finalized from its current state");
        }
        let duration_ms: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(end_ms), 0) FROM meeting_chunks WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_one(&mut *transaction)
        .await?;
        let now = unix_time();
        sqlx::query(
            "UPDATE meetings
             SET state = 'processing', ended_at = ?, duration_ms = ?,
                 final_audio_path = ?, final_audio_size_bytes = ?,
                 final_audio_checksum = ?, error_stage = NULL, error_message = NULL,
                 updated_at = ?
             WHERE id = ? AND user_id = ?",
        )
        .bind(ended_at)
        .bind(duration_ms)
        .bind(&audio.relative_path)
        .bind(audio.size_bytes)
        .bind(&audio.checksum)
        .bind(now)
        .bind(meeting_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, 'transcript', 'pending', ?)
             ON CONFLICT(meeting_id, stage) DO UPDATE
             SET status = CASE
                    WHEN meeting_processing_jobs.status = 'completed' THEN 'completed'
                    ELSE 'pending'
                 END,
                 updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn owned_final_audio(
        &self,
        user_id: &str,
        meeting_id: &str,
    ) -> anyhow::Result<Option<FinalAudioMetadata>> {
        let row = sqlx::query(
            "SELECT final_audio_path, final_audio_size_bytes, final_audio_checksum
             FROM meetings WHERE id = ? AND user_id = ?",
        )
        .bind(meeting_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let (Some(relative_path), Some(size_bytes), Some(checksum)) = (
            row.get::<Option<String>, _>("final_audio_path"),
            row.get::<Option<i64>, _>("final_audio_size_bytes"),
            row.get::<Option<String>, _>("final_audio_checksum"),
        ) else {
            return Ok(None);
        };
        if !safe_relative_path(&relative_path)
            || relative_path != format!("{meeting_id}/recording.m4a")
        {
            bail!("unsafe stored final audio path");
        }
        Ok(Some(FinalAudioMetadata {
            relative_path,
            size_bytes,
            checksum,
        }))
    }

    pub async fn transition(
        &self,
        meeting_id: &str,
        next: MeetingState,
        ended_at: Option<f64>,
    ) -> anyhow::Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let Some(row) = sqlx::query("SELECT state, started_at FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&mut *transaction)
            .await?
        else {
            transaction.rollback().await?;
            return Ok(false);
        };
        let current = MeetingState::parse(&row.get::<String, _>("state"))?;
        if !current.can_transition_to(next) {
            bail!("invalid meeting transition: {current:?} -> {next:?}");
        }
        let started_at = row.get::<f64, _>("started_at");
        let duration_ms = match ended_at {
            Some(value) if value.is_finite() && value >= started_at => {
                let row = sqlx::query(
                    "SELECT COALESCE(MAX(end_ms), 0) AS duration_ms
                     FROM meeting_chunks WHERE meeting_id = ?",
                )
                .bind(meeting_id)
                .fetch_one(&mut *transaction)
                .await?;
                Some(row.get::<i64, _>("duration_ms"))
            }
            Some(_) => bail!("ended_at must not precede started_at"),
            None => None,
        };
        sqlx::query(
            "UPDATE meetings
             SET state = ?, ended_at = COALESCE(?, ended_at),
                 duration_ms = COALESCE(?, duration_ms), updated_at = ?
             WHERE id = ?",
        )
        .bind(next.as_str())
        .bind(ended_at)
        .bind(duration_ms)
        .bind(unix_time())
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn replace_transcript(
        &self,
        meeting_id: &str,
        segments: &[TranscriptSegment],
    ) -> anyhow::Result<()> {
        let mut ids = HashSet::new();
        for segment in segments {
            if segment.id < 0
                || segment.start_ms < 0
                || segment.end_ms <= segment.start_ms
                || !ids.insert(segment.id)
            {
                bail!("invalid transcript segment timing or id");
            }
        }
        let mut transaction = self.pool.begin().await?;
        ensure_meeting_exists(&mut transaction, meeting_id).await?;
        sqlx::query("DELETE FROM meeting_transcript_segments WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        for segment in segments {
            sqlx::query(
                "INSERT INTO meeting_transcript_segments(
                    meeting_id, id, start_ms, end_ms, text, provisional
                 ) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(meeting_id)
            .bind(segment.id)
            .bind(segment.start_ms)
            .bind(segment.end_ms)
            .bind(&segment.text)
            .bind(segment.provisional)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn replace_artifact(
        &self,
        meeting_id: &str,
        title: &str,
        summary: &str,
        todos: &[MeetingTodo],
    ) -> anyhow::Result<()> {
        let mut ids = HashSet::new();
        for todo in todos {
            if todo.id.trim().is_empty() || todo.text.trim().is_empty() || !ids.insert(&todo.id) {
                bail!("meeting todo id and text must be non-empty and unique");
            }
            match (todo.source_start_ms, todo.source_end_ms) {
                (None, None) => {}
                (Some(start), Some(end)) if start >= 0 && end > start => {}
                _ => bail!("meeting todo source timing must be a valid complete range"),
            }
        }
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        ensure_meeting_exists(&mut transaction, meeting_id).await?;
        sqlx::query("UPDATE meetings SET title = ?, summary = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(summary)
            .bind(now)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM meeting_todos WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        for todo in todos {
            sqlx::query(
                "INSERT INTO meeting_todos(
                    meeting_id, id, text, completed, source_start_ms, source_end_ms,
                    created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(meeting_id)
            .bind(&todo.id)
            .bind(&todo.text)
            .bind(todo.completed)
            .bind(todo.source_start_ms)
            .bind(todo.source_end_ms)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn delete_owned(&self, user_id: &str, meeting_id: &str) -> anyhow::Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query("DELETE FROM meetings WHERE user_id = ? AND id = ?")
            .bind(user_id)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn update_todo_completed(
        &self,
        user_id: &str,
        meeting_id: &str,
        todo_id: &str,
        completed: bool,
    ) -> anyhow::Result<Option<MeetingTodo>> {
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "UPDATE meeting_todos
             SET completed = ?, updated_at = ?
             WHERE meeting_id = ? AND id = ?
               AND EXISTS (
                   SELECT 1 FROM meetings
                   WHERE meetings.id = meeting_todos.meeting_id
                     AND meetings.user_id = ?
               )
             RETURNING id, text, completed, source_start_ms, source_end_ms",
        )
        .bind(completed)
        .bind(now)
        .bind(meeting_id)
        .bind(todo_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(None);
        };
        sqlx::query("UPDATE meetings SET updated_at = ? WHERE id = ? AND user_id = ?")
            .bind(now)
            .bind(meeting_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(MeetingTodo {
            id: row.get("id"),
            text: row.get("text"),
            completed: row.get("completed"),
            source_start_ms: row.get("source_start_ms"),
            source_end_ms: row.get("source_end_ms"),
        }))
    }

    async fn transcript(&self, meeting_id: &str) -> anyhow::Result<Vec<TranscriptSegment>> {
        let rows = sqlx::query(
            "SELECT id, start_ms, end_ms, text, provisional
             FROM meeting_transcript_segments WHERE meeting_id = ? ORDER BY start_ms, id",
        )
        .bind(meeting_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| TranscriptSegment {
                id: row.get("id"),
                start_ms: row.get("start_ms"),
                end_ms: row.get("end_ms"),
                text: row.get("text"),
                provisional: row.get("provisional"),
            })
            .collect())
    }

    async fn todos(&self, meeting_id: &str) -> anyhow::Result<Vec<MeetingTodo>> {
        let rows = sqlx::query(
            "SELECT id, text, completed, source_start_ms, source_end_ms
             FROM meeting_todos WHERE meeting_id = ? ORDER BY created_at, id",
        )
        .bind(meeting_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| MeetingTodo {
                id: row.get("id"),
                text: row.get("text"),
                completed: row.get("completed"),
                source_start_ms: row.get("source_start_ms"),
                source_end_ms: row.get("source_end_ms"),
            })
            .collect())
    }
}

async fn ensure_meeting_exists(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    meeting_id: &str,
) -> anyhow::Result<()> {
    let exists = sqlx::query("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(&mut **transaction)
        .await?
        .is_some();
    if !exists {
        bail!("meeting not found");
    }
    Ok(())
}

async fn ensure_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    if columns
        .iter()
        .any(|row| row.get::<String, _>("name") == column)
    {
        return Ok(());
    }
    sqlx::query(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn meeting_from_row(row: SqliteRow) -> anyhow::Result<Meeting> {
    let error_stage = row
        .get::<Option<String>, _>("error_stage")
        .map(|value| ProcessingStage::parse(&value))
        .transpose()?;
    Ok(Meeting {
        id: row.get("id"),
        state: MeetingState::parse(&row.get::<String, _>("state"))?,
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        duration_ms: row.get("duration_ms"),
        title: row.get("title"),
        summary: row.get("summary"),
        error_stage,
        error_message: row.get("error_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        transcript: Vec::new(),
        todos: Vec::new(),
    })
}

fn unix_time() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::Row;
    use tokio::sync::Barrier;

    use crate::{
        context::ContextStore,
        meeting::types::{
            ChunkWrite, FinalAudioMetadata, MeetingState, MeetingTodo, TranscriptSegment,
        },
    };

    use super::MeetingStore;

    async fn test_store() -> (tempfile::TempDir, ContextStore, MeetingStore) {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3"))
            .await
            .unwrap();
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await.unwrap();
        (directory, context, store)
    }

    #[tokio::test]
    async fn create_is_idempotent_and_reads_are_owner_scoped() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "idem-1", 1_700_000_000.0).await?;
        assert_eq!(meeting.state, MeetingState::Recording);
        assert_eq!(
            store.create("user-a", "idem-1", 1_700_000_000.0).await?.id,
            meeting.id
        );
        assert!(store.get_owned("user-b", &meeting.id).await?.is_none());
        assert_eq!(store.list("user-b").await?, Vec::new());
        assert_eq!(store.list("user-a").await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn transition_excludes_paused_wall_clock_from_recorded_duration() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "lifecycle", 100.0).await?;
        store
            .record_chunk(&meeting.id, 0, 0, 10_000, "first", 100)
            .await?;
        store
            .record_chunk(&meeting.id, 1, 10_000, 20_000, "second", 100)
            .await?;
        assert!(
            store
                .transition(&meeting.id, MeetingState::Paused, None)
                .await?
        );
        assert_eq!(
            store.get_owned("user-a", &meeting.id).await?.unwrap().state,
            MeetingState::Paused
        );
        assert!(
            store
                .transition(&meeting.id, MeetingState::Recording, None)
                .await?
        );
        assert!(
            store
                .transition(&meeting.id, MeetingState::Uploading, Some(130.0))
                .await?
        );
        assert!(
            store
                .transition(&meeting.id, MeetingState::Processing, None)
                .await?
        );
        assert!(
            store
                .transition(&meeting.id, MeetingState::Completed, None)
                .await?
        );
        let completed = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(completed.ended_at, Some(130.0));
        // Thirty seconds elapsed on the wall clock, but the chunk timeline excludes
        // the ten-second pause and is the authoritative recorded-audio duration.
        assert_eq!(completed.duration_ms, Some(20_000));
        assert!(
            store
                .transition(&meeting.id, MeetingState::Recording, None)
                .await
                .is_err()
        );
        assert!(
            !store
                .transition("missing", MeetingState::Paused, None)
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn chunk_sequence_is_idempotent_but_checksum_conflicts() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "chunks", 100.0).await?;
        assert_eq!(
            store
                .record_chunk(&meeting.id, 0, 0, 15_000, "abc", 100)
                .await?,
            ChunkWrite::Inserted
        );
        assert_eq!(
            store
                .record_chunk(&meeting.id, 0, 0, 15_000, "abc", 100)
                .await?,
            ChunkWrite::Existing
        );
        assert_eq!(
            store
                .record_chunk(&meeting.id, 0, 0, 15_000, "other", 100)
                .await?,
            ChunkWrite::Conflict
        );
        store
            .record_chunk(&meeting.id, 2, 30_000, 45_000, "ghi", 100)
            .await?;
        assert_eq!(store.missing_sequences(&meeting.id, 3).await?, vec![1, 3]);
        Ok(())
    }

    #[tokio::test]
    async fn verified_chunk_path_is_bound_to_its_meeting_and_sequence() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "bound-chunk", 100.0).await?;
        let other = store.create("user-a", "other-chunk", 100.0).await?;

        let error = store
            .record_verified_chunk(
                &meeting.id,
                0,
                0,
                1_000,
                &"a".repeat(64),
                100,
                &format!("{}/chunks/0.m4a", other.id),
            )
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "invalid verified meeting chunk path");
        Ok(())
    }

    #[tokio::test]
    async fn verified_chunk_path_is_bound_to_its_meeting_and_sequence_on_read() -> anyhow::Result<()>
    {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "read-bound-chunk", 100.0).await?;
        let other = store.create("user-b", "foreign-chunk", 100.0).await?;
        store
            .record_verified_chunk(
                &meeting.id,
                0,
                0,
                1_000,
                &"d".repeat(64),
                100,
                &format!("{}/chunks/0.m4a", meeting.id),
            )
            .await?;
        sqlx::query("UPDATE meeting_chunks SET content_path = ? WHERE meeting_id = ?")
            .bind(format!("{}/chunks/0.m4a", other.id))
            .bind(&meeting.id)
            .execute(&context.pool_clone())
            .await?;

        let error = store.verified_chunks(&meeting.id, 0).await.unwrap_err();

        assert_eq!(error.to_string(), "unsafe stored meeting chunk path");
        Ok(())
    }

    #[tokio::test]
    async fn final_audio_path_is_bound_to_its_owned_meeting_on_write() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "bound-final", 100.0).await?;
        let other = store.create("user-a", "other-final", 100.0).await?;
        let audio = FinalAudioMetadata {
            relative_path: format!("{}/recording.m4a", other.id),
            size_bytes: 100,
            checksum: "b".repeat(64),
        };

        let error = store
            .finalize_owned_upload("user-a", &meeting.id, 110.0, &audio)
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "invalid final meeting audio path");
        assert_eq!(
            store.get_owned("user-a", &meeting.id).await?.unwrap().state,
            MeetingState::Recording
        );
        Ok(())
    }

    #[tokio::test]
    async fn final_audio_path_is_bound_to_its_owned_meeting_on_read() -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "read-bound-final", 100.0).await?;
        let other = store.create("user-b", "foreign-final", 100.0).await?;
        sqlx::query(
            "UPDATE meetings
             SET final_audio_path = ?, final_audio_size_bytes = 100, final_audio_checksum = ?
             WHERE id = ?",
        )
        .bind(format!("{}/recording.m4a", other.id))
        .bind("c".repeat(64))
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;

        let error = store
            .owned_final_audio("user-a", &meeting.id)
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "unsafe stored final audio path");
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_identical_chunk_retries_are_idempotent() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "concurrent-chunks", 100.0).await?;
        let concurrency = 8;
        let barrier = Arc::new(Barrier::new(concurrency));
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let store = store.clone();
            let meeting_id = meeting.id.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                store
                    .record_chunk(&meeting_id, 0, 0, 15_000, "same", 100)
                    .await
            }));
        }

        let mut inserted = 0;
        let mut existing = 0;
        for task in tasks {
            match task.await?? {
                ChunkWrite::Inserted => inserted += 1,
                ChunkWrite::Existing => existing += 1,
                ChunkWrite::Conflict => anyhow::bail!("identical retry must not conflict"),
            }
        }
        assert_eq!(inserted, 1);
        assert_eq!(existing, concurrency - 1);
        Ok(())
    }

    #[tokio::test]
    async fn transcript_replacement_validates_timing_and_replaces_provisional_rows()
    -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "transcript", 100.0).await?;
        store
            .replace_transcript(
                &meeting.id,
                &[TranscriptSegment {
                    id: 1,
                    start_ms: 0,
                    end_ms: 15_000,
                    text: "临时文本".to_owned(),
                    provisional: true,
                }],
            )
            .await?;
        store
            .replace_transcript(
                &meeting.id,
                &[
                    TranscriptSegment {
                        id: 2,
                        start_ms: 0,
                        end_ms: 8_000,
                        text: "最终文本一".to_owned(),
                        provisional: false,
                    },
                    TranscriptSegment {
                        id: 3,
                        start_ms: 8_000,
                        end_ms: 15_000,
                        text: "最终文本二".to_owned(),
                        provisional: false,
                    },
                ],
            )
            .await?;
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.transcript.len(), 2);
        assert_eq!(detail.transcript[0].text, "最终文本一");
        assert!(detail.transcript.iter().all(|segment| !segment.provisional));
        let invalid = [TranscriptSegment {
            id: 4,
            start_ms: 20,
            end_ms: 10,
            text: "无效".to_owned(),
            provisional: false,
        }];
        assert!(
            store
                .replace_transcript(&meeting.id, &invalid)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .get_owned("user-a", &meeting.id)
                .await?
                .unwrap()
                .transcript
                .len(),
            2
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_todos_are_meeting_local_and_delete_cascades() -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "artifact", 100.0).await?;
        store
            .record_chunk(&meeting.id, 0, 0, 15_000, "abc", 100)
            .await?;
        store
            .replace_transcript(
                &meeting.id,
                &[TranscriptSegment {
                    id: 1,
                    start_ms: 0,
                    end_ms: 15_000,
                    text: "讨论发布安排".to_owned(),
                    provisional: false,
                }],
            )
            .await?;
        store
            .replace_artifact(
                &meeting.id,
                "发布会",
                "讨论发布安排",
                &[MeetingTodo {
                    id: "meeting-todo-1".to_owned(),
                    text: "准备演示".to_owned(),
                    completed: false,
                    source_start_ms: Some(1_000),
                    source_end_ms: Some(2_000),
                }],
            )
            .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, updated_at)
             VALUES (?, 'organization', ?)",
        )
        .bind(&meeting.id)
        .bind(100.0)
        .execute(&context.pool_clone())
        .await?;
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.title.as_deref(), Some("发布会"));
        assert_eq!(detail.summary.as_deref(), Some("讨论发布安排"));
        assert_eq!(detail.todos.len(), 1);
        assert_eq!(detail.todos[0].source_start_ms, Some(1_000));
        let global_todo_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM todos")
            .fetch_one(&context.pool_clone())
            .await?
            .get("count");
        assert_eq!(global_todo_count, 0);
        assert!(!store.delete_owned("user-b", &meeting.id).await?);
        assert!(store.delete_owned("user-a", &meeting.id).await?);
        assert!(store.get_owned("user-a", &meeting.id).await?.is_none());
        for table in [
            "meeting_chunks",
            "meeting_transcript_segments",
            "meeting_todos",
            "meeting_processing_jobs",
        ] {
            let count: i64 = sqlx::query(&format!(
                "SELECT COUNT(*) AS count FROM {table} WHERE meeting_id = ?"
            ))
            .bind(&meeting.id)
            .fetch_one(&context.pool_clone())
            .await?
            .get("count");
            assert_eq!(count, 0, "{table} should cascade");
        }
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&context.pool_clone())
            .await?;
        assert!(violations.is_empty());
        Ok(())
    }
}
