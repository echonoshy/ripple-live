use std::{
    collections::HashSet,
    path::{Component, Path},
    time::SystemTime,
};

use anyhow::{Context, bail};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::storage::VerifiedLegacyFinalization;
use super::types::{
    ChunkWrite, FinalAudioMetadata, FinalizeOutcome, MAX_MEETING_CHUNK_SEQUENCE,
    MAX_MEETING_DURATION_MS, Meeting, MeetingArtifact, MeetingState, MeetingTodo, ProcessingStage,
    RetryStageOutcome, StoredChunkMetadata, TranscriptJobClaim, TranscriptSegment,
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
                final_sequence INTEGER CHECK(final_sequence IS NULL OR (final_sequence >= 0 AND final_sequence <= 100000)),
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
        ensure_column(
            &self.pool,
            "meetings",
            "final_sequence",
            "INTEGER CHECK(final_sequence IS NULL OR (final_sequence >= 0 AND final_sequence <= 100000))",
        )
        .await?;
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
        if sequence > MAX_MEETING_CHUNK_SEQUENCE {
            bail!("meeting chunk sequence exceeds safety limit");
        }
        if sequence < 0 || start_ms < 0 || end_ms <= start_ms || size_bytes < 0 {
            bail!("invalid meeting chunk metadata");
        }
        if end_ms - start_ms > MAX_MEETING_DURATION_MS {
            return Ok(ChunkWrite::DurationExceeded);
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
        if last_sequence > MAX_MEETING_CHUNK_SEQUENCE {
            bail!("meeting final sequence exceeds safety limit");
        }
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
        if sequence > MAX_MEETING_CHUNK_SEQUENCE {
            bail!("meeting chunk sequence exceeds safety limit");
        }
        if sequence < 0 || start_ms < 0 || end_ms <= start_ms || size_bytes < 0 {
            bail!("invalid meeting chunk metadata");
        }
        if end_ms - start_ms > MAX_MEETING_DURATION_MS {
            return Ok(ChunkWrite::DurationExceeded);
        }
        if checksum.trim().is_empty() || !safe_relative_path(relative_path) {
            bail!("invalid verified meeting chunk metadata");
        }
        if relative_path != format!("{meeting_id}/chunks/{sequence}.m4a") {
            bail!("invalid verified meeting chunk path");
        }
        let mut transaction = self.pool.begin().await?;
        let meeting = sqlx::query("SELECT state, final_sequence FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(&mut *transaction)
            .await?;
        let Some(meeting) = meeting else {
            transaction.rollback().await?;
            bail!("meeting not found");
        };
        let state = MeetingState::parse(&meeting.get::<String, _>("state"))?;
        let final_sequence = meeting.get::<Option<i64>, _>("final_sequence");
        if final_sequence.is_some_and(|boundary| sequence > boundary) {
            transaction.rollback().await?;
            return Ok(ChunkWrite::Conflict);
        }
        let only_existing = matches!(state, MeetingState::Processing | MeetingState::Completed);
        if !only_existing {
            let result = sqlx::query(
                "INSERT INTO meeting_chunks(
                    meeting_id, sequence, start_ms, end_ms, checksum, size_bytes,
                    content_path, verified, created_at
                 )
                 SELECT ?, ?, ?, ?, ?, ?, ?, 1, ?
                 WHERE (
                     SELECT COALESCE(SUM(end_ms - start_ms), 0)
                     FROM meeting_chunks WHERE meeting_id = ?
                 ) + (? - ?) <= ?
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
            .bind(meeting_id)
            .bind(end_ms)
            .bind(start_ms)
            .bind(MAX_MEETING_DURATION_MS)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() == 1 {
                transaction.commit().await?;
                return Ok(ChunkWrite::Inserted);
            }
        }

        let row = sqlx::query(
            "SELECT start_ms, end_ms, checksum, size_bytes, content_path, verified
             FROM meeting_chunks WHERE meeting_id = ? AND sequence = ?",
        )
        .bind(meeting_id)
        .bind(sequence)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(if only_existing {
                ChunkWrite::Conflict
            } else {
                ChunkWrite::DurationExceeded
            });
        };
        let identical = row.get::<i64, _>("start_ms") == start_ms
            && row.get::<i64, _>("end_ms") == end_ms
            && row.get::<String, _>("checksum") == checksum
            && row.get::<i64, _>("size_bytes") == size_bytes;
        if !identical {
            transaction.rollback().await?;
            return Ok(ChunkWrite::Conflict);
        }
        let stored_path = row.get::<Option<String>, _>("content_path");
        if stored_path
            .as_deref()
            .is_some_and(|stored_path| stored_path != relative_path)
        {
            transaction.rollback().await?;
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
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(ChunkWrite::Existing)
    }

    pub async fn missing_verified_sequences(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<i64>> {
        if last_sequence > MAX_MEETING_CHUNK_SEQUENCE {
            bail!("meeting final sequence exceeds safety limit");
        }
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
            "SELECT sequence, start_ms, end_ms, content_path, size_bytes, checksum
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
                start_ms: row.get("start_ms"),
                end_ms: row.get("end_ms"),
                relative_path,
                size_bytes: row.get("size_bytes"),
                checksum: row.get("checksum"),
            });
        }
        Ok(chunks)
    }

    pub async fn recorded_duration_ms(&self, meeting_id: &str) -> anyhow::Result<i64> {
        let duration = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(end_ms), 0) FROM meeting_chunks WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(duration)
    }

    pub async fn enqueue_transcript_job(&self, meeting_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO meeting_processing_jobs(
                meeting_id, stage, status, updated_at
             ) VALUES (?, 'transcript', 'pending', ?)",
        )
        .bind(meeting_id)
        .bind(unix_time())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn claim_transcript_job(
        &self,
        meeting_id: &str,
    ) -> anyhow::Result<TranscriptJobClaim> {
        let now = unix_time();
        let stale_before = now - 900.0;
        let claimed = sqlx::query(
            "UPDATE meeting_processing_jobs
             SET status = 'running', attempt = attempt + 1,
                 diagnostic_error = NULL, updated_at = ?
             WHERE meeting_id = ? AND stage = 'transcript'
               AND (status IN ('pending', 'failed')
                    OR (status = 'running' AND updated_at < ?))
             RETURNING attempt",
        )
        .bind(now)
        .bind(meeting_id)
        .bind(stale_before)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = claimed {
            return Ok(TranscriptJobClaim::Claimed {
                attempt: row.get("attempt"),
            });
        }
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM meeting_processing_jobs
             WHERE meeting_id = ? AND stage = 'transcript'",
        )
        .bind(meeting_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match status.as_deref() {
            Some("completed") => TranscriptJobClaim::Completed,
            Some("running") | Some("pending") | Some("failed") => TranscriptJobClaim::Busy,
            _ => TranscriptJobClaim::Missing,
        })
    }

    pub async fn heartbeat_transcript_job(
        &self,
        meeting_id: &str,
        attempt: i64,
    ) -> anyhow::Result<bool> {
        let updated = sqlx::query(
            "UPDATE meeting_processing_jobs SET updated_at = ?
             WHERE meeting_id = ? AND stage = 'transcript'
               AND status = 'running' AND attempt = ?",
        )
        .bind(unix_time())
        .bind(meeting_id)
        .bind(attempt)
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn fail_transcript_job(&self, meeting_id: &str, attempt: i64) -> anyhow::Result<()> {
        const SAFE_ERROR: &str = "transcript processing failed";
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let failed = sqlx::query(
            "UPDATE meeting_processing_jobs
             SET status = 'failed', diagnostic_error = ?, updated_at = ?
             WHERE meeting_id = ? AND stage = 'transcript'
               AND status = 'running' AND attempt = ?",
        )
        .bind(SAFE_ERROR)
        .bind(now)
        .bind(meeting_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        if failed.rows_affected() == 1 {
            sqlx::query(
                "UPDATE meetings SET error_stage = 'transcript', error_message = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(SAFE_ERROR)
            .bind(now)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn complete_transcript_job(
        &self,
        meeting_id: &str,
        attempt: i64,
        segments: &[TranscriptSegment],
    ) -> anyhow::Result<()> {
        validate_transcript_segments(segments, true)?;
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        ensure_meeting_exists(&mut transaction, meeting_id).await?;
        let running = sqlx::query(
            "UPDATE meeting_processing_jobs
             SET status = 'completed', diagnostic_error = NULL, updated_at = ?
             WHERE meeting_id = ? AND stage = 'transcript'
               AND status = 'running' AND attempt = ?",
        )
        .bind(now)
        .bind(meeting_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        if running.rows_affected() != 1 {
            bail!("transcript job was not running");
        }
        sqlx::query("DELETE FROM meeting_transcript_segments WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        for segment in segments {
            sqlx::query(
                "INSERT INTO meeting_transcript_segments(
                    meeting_id, id, start_ms, end_ms, text, provisional
                 ) VALUES (?, ?, ?, ?, ?, 0)",
            )
            .bind(meeting_id)
            .bind(segment.id)
            .bind(segment.start_ms)
            .bind(segment.end_ms)
            .bind(&segment.text)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, 'organization', 'pending', ?)
             ON CONFLICT(meeting_id, stage) DO UPDATE SET
                status = CASE WHEN meeting_processing_jobs.status = 'completed'
                              THEN 'completed' ELSE 'pending' END,
                diagnostic_error = NULL,
                updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE meetings
             SET error_stage = CASE WHEN error_stage = 'transcript' THEN NULL ELSE error_stage END,
                 error_message = CASE WHEN error_stage = 'transcript' THEN NULL ELSE error_message END,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn final_transcript(
        &self,
        meeting_id: &str,
    ) -> anyhow::Result<Vec<TranscriptSegment>> {
        let segments = self.transcript(meeting_id).await?;
        if segments.iter().any(|segment| segment.provisional) {
            bail!("meeting final transcript is not complete");
        }
        Ok(segments)
    }

    pub async fn claim_organization_job(
        &self,
        meeting_id: &str,
    ) -> anyhow::Result<TranscriptJobClaim> {
        claim_processing_job(&self.pool, meeting_id, "organization").await
    }

    pub async fn heartbeat_organization_job(
        &self,
        meeting_id: &str,
        attempt: i64,
    ) -> anyhow::Result<bool> {
        heartbeat_processing_job(&self.pool, meeting_id, "organization", attempt).await
    }

    pub async fn fail_organization_job(
        &self,
        meeting_id: &str,
        attempt: i64,
    ) -> anyhow::Result<()> {
        const SAFE_ERROR: &str = "meeting organization failed";
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let failed = sqlx::query(
            "UPDATE meeting_processing_jobs
             SET status = 'failed', diagnostic_error = ?, updated_at = ?
             WHERE meeting_id = ? AND stage = 'organization'
               AND status = 'running' AND attempt = ?",
        )
        .bind(SAFE_ERROR)
        .bind(now)
        .bind(meeting_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        if failed.rows_affected() == 1 {
            sqlx::query(
                "UPDATE meetings SET error_stage = 'organization', error_message = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(SAFE_ERROR)
            .bind(now)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn complete_organization_job(
        &self,
        meeting_id: &str,
        attempt: i64,
        artifact: &MeetingArtifact,
    ) -> anyhow::Result<()> {
        validate_meeting_artifact(artifact)?;
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        ensure_meeting_exists(&mut transaction, meeting_id).await?;
        let completed = sqlx::query(
            "UPDATE meeting_processing_jobs
             SET status = 'completed', diagnostic_error = NULL, updated_at = ?
             WHERE meeting_id = ? AND stage = 'organization'
               AND status = 'running' AND attempt = ?",
        )
        .bind(now)
        .bind(meeting_id)
        .bind(attempt)
        .execute(&mut *transaction)
        .await?;
        if completed.rows_affected() != 1 {
            bail!("organization job was not running");
        }
        sqlx::query(
            "UPDATE meetings
             SET title = ?, summary = ?, state = 'completed',
                 error_stage = NULL, error_message = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(artifact.title.trim())
        .bind(artifact.summary.trim())
        .bind(now)
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM meeting_todos WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        for todo in &artifact.todos {
            sqlx::query(
                "INSERT INTO meeting_todos(
                    meeting_id, id, text, completed, source_start_ms, source_end_ms,
                    created_at, updated_at
                 ) VALUES (?, ?, ?, 0, ?, ?, ?, ?)",
            )
            .bind(meeting_id)
            .bind(Uuid::new_v4().to_string())
            .bind(todo.text.trim())
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

    pub async fn retry_stage_owned(
        &self,
        user_id: &str,
        meeting_id: &str,
        stage: ProcessingStage,
    ) -> anyhow::Result<RetryStageOutcome> {
        if stage == ProcessingStage::Upload {
            return Ok(RetryStageOutcome::Unavailable);
        }
        let stage_name = match stage {
            ProcessingStage::Transcript => "transcript",
            ProcessingStage::Organization => "organization",
            ProcessingStage::Upload => unreachable!(),
        };
        let now = unix_time();
        let stale_before = now - 900.0;
        let mut transaction = self.pool.begin().await?;
        let meeting =
            sqlx::query("SELECT final_audio_path FROM meetings WHERE id = ? AND user_id = ?")
                .bind(meeting_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(meeting) = meeting else {
            transaction.rollback().await?;
            return Ok(RetryStageOutcome::NotFound);
        };
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM meeting_processing_jobs WHERE meeting_id = ? AND stage = ?",
        )
        .bind(meeting_id)
        .bind(stage_name)
        .fetch_optional(&mut *transaction)
        .await?;
        if status.as_deref() == Some("completed") {
            transaction.rollback().await?;
            return Ok(RetryStageOutcome::Completed);
        }
        if stage == ProcessingStage::Transcript
            && meeting
                .get::<Option<String>, _>("final_audio_path")
                .is_none()
        {
            transaction.rollback().await?;
            return Ok(RetryStageOutcome::Unavailable);
        }
        if stage == ProcessingStage::Organization {
            let transcript_complete = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM meeting_processing_jobs
                 WHERE meeting_id = ? AND stage = 'transcript' AND status = 'completed'",
            )
            .bind(meeting_id)
            .fetch_one(&mut *transaction)
            .await?
                == 1;
            if !transcript_complete {
                transaction.rollback().await?;
                return Ok(RetryStageOutcome::Unavailable);
            }
        }
        let busy = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM meeting_processing_jobs
             WHERE meeting_id = ? AND stage = ? AND status = 'running' AND updated_at >= ?",
        )
        .bind(meeting_id)
        .bind(stage_name)
        .bind(stale_before)
        .fetch_one(&mut *transaction)
        .await?
            == 1;
        if busy {
            transaction.rollback().await?;
            return Ok(RetryStageOutcome::Busy);
        }
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, ?, 'pending', ?)
             ON CONFLICT(meeting_id, stage) DO UPDATE SET
                status = 'pending', diagnostic_error = NULL, updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(stage_name)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE meetings SET error_stage = NULL, error_message = NULL, state = 'processing', updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(RetryStageOutcome::Queued)
    }

    pub async fn claim_finalization(
        &self,
        user_id: &str,
        meeting_id: &str,
        last_sequence: i64,
        ended_at: f64,
    ) -> anyhow::Result<FinalizeOutcome> {
        if !(0..=MAX_MEETING_CHUNK_SEQUENCE).contains(&last_sequence) {
            bail!("meeting final sequence exceeds safety limit");
        }
        if !ended_at.is_finite() {
            bail!("invalid meeting end timestamp");
        }
        let actual_duration_ms = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(meeting_chunks.end_ms - meeting_chunks.start_ms), 0)
             FROM meetings
             LEFT JOIN meeting_chunks
               ON meeting_chunks.meeting_id = meetings.id
              AND meeting_chunks.sequence <= ?
             WHERE meetings.id = ? AND meetings.user_id = ?
             GROUP BY meetings.id",
        )
        .bind(last_sequence)
        .bind(meeting_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(actual_duration_ms) = actual_duration_ms else {
            return Ok(FinalizeOutcome::NotFound);
        };
        if actual_duration_ms > MAX_MEETING_DURATION_MS {
            return Ok(FinalizeOutcome::DurationExceeded);
        }
        let now = unix_time();
        let claimed = sqlx::query(
            "UPDATE meetings
             SET final_sequence = ?, ended_at = ?, state = 'uploading', updated_at = ?
             WHERE id = ? AND user_id = ? AND final_sequence IS NULL
               AND state IN ('recording', 'paused', 'interrupted', 'uploading')
               AND started_at <= ?
               AND NOT EXISTS (
                   SELECT 1 FROM meeting_chunks
                   WHERE meeting_chunks.meeting_id = meetings.id AND sequence > ?
               )",
        )
        .bind(last_sequence)
        .bind(ended_at)
        .bind(now)
        .bind(meeting_id)
        .bind(user_id)
        .bind(ended_at)
        .bind(last_sequence)
        .execute(&self.pool)
        .await?;
        if claimed.rows_affected() == 1 {
            return Ok(FinalizeOutcome::Pending);
        }

        let row = sqlx::query(
            "SELECT state, started_at, final_sequence, final_audio_path,
                    final_audio_size_bytes, final_audio_checksum
             FROM meetings WHERE id = ? AND user_id = ?",
        )
        .bind(meeting_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(FinalizeOutcome::NotFound);
        };
        if ended_at < row.get::<f64, _>("started_at") {
            bail!("ended_at must not precede started_at");
        }
        let state = MeetingState::parse(&row.get::<String, _>("state"))?;
        let final_sequence = row.get::<Option<i64>, _>("final_sequence");
        if final_sequence.is_none()
            && matches!(state, MeetingState::Processing | MeetingState::Completed)
        {
            let (Some(relative_path), Some(size_bytes), Some(checksum)) = (
                row.get::<Option<String>, _>("final_audio_path"),
                row.get::<Option<i64>, _>("final_audio_size_bytes"),
                row.get::<Option<String>, _>("final_audio_checksum"),
            ) else {
                return Ok(FinalizeOutcome::Conflict);
            };
            if relative_path != format!("{meeting_id}/recording.m4a")
                || !safe_relative_path(&relative_path)
                || size_bytes < 0
                || checksum.trim().is_empty()
            {
                return Ok(FinalizeOutcome::Conflict);
            }
            return Ok(FinalizeOutcome::LegacyVerificationRequired(
                FinalAudioMetadata {
                    relative_path,
                    size_bytes,
                    checksum,
                },
            ));
        }
        if final_sequence != Some(last_sequence) {
            return Ok(FinalizeOutcome::Conflict);
        }
        Ok(match state {
            MeetingState::Uploading => FinalizeOutcome::Pending,
            MeetingState::Processing | MeetingState::Completed
                if row.get::<Option<String>, _>("final_audio_path").is_some() =>
            {
                FinalizeOutcome::Finalized(state)
            }
            _ => FinalizeOutcome::Conflict,
        })
    }

    pub async fn recover_legacy_finalization(
        &self,
        user_id: &str,
        proof: VerifiedLegacyFinalization,
    ) -> anyhow::Result<FinalizeOutcome> {
        let VerifiedLegacyFinalization {
            meeting_id,
            last_sequence,
            audio,
        } = proof;
        if !(0..=MAX_MEETING_CHUNK_SEQUENCE).contains(&last_sequence)
            || audio.relative_path != format!("{meeting_id}/recording.m4a")
            || !safe_relative_path(&audio.relative_path)
            || audio.size_bytes < 0
            || audio.checksum.trim().is_empty()
        {
            bail!("invalid verified legacy finalization");
        }
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let recovered = sqlx::query(
            "UPDATE meetings
             SET final_sequence = ?, updated_at = ?
             WHERE id = ? AND user_id = ? AND final_sequence IS NULL
               AND state IN ('processing', 'completed')
               AND final_audio_path = ? AND final_audio_size_bytes = ?
               AND final_audio_checksum = ?",
        )
        .bind(last_sequence)
        .bind(now)
        .bind(&meeting_id)
        .bind(user_id)
        .bind(&audio.relative_path)
        .bind(audio.size_bytes)
        .bind(&audio.checksum)
        .execute(&mut *transaction)
        .await?;
        if recovered.rows_affected() == 1 {
            let state: String = sqlx::query_scalar("SELECT state FROM meetings WHERE id = ?")
                .bind(&meeting_id)
                .fetch_one(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Ok(FinalizeOutcome::Finalized(MeetingState::parse(&state)?));
        }

        let row = sqlx::query(
            "SELECT state, final_sequence, final_audio_path,
                    final_audio_size_bytes, final_audio_checksum
             FROM meetings WHERE id = ? AND user_id = ?",
        )
        .bind(&meeting_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(FinalizeOutcome::NotFound);
        };
        let state = MeetingState::parse(&row.get::<String, _>("state"))?;
        let identical = row.get::<Option<i64>, _>("final_sequence") == Some(last_sequence)
            && row.get::<Option<String>, _>("final_audio_path").as_deref()
                == Some(audio.relative_path.as_str())
            && row.get::<Option<i64>, _>("final_audio_size_bytes") == Some(audio.size_bytes)
            && row
                .get::<Option<String>, _>("final_audio_checksum")
                .as_deref()
                == Some(audio.checksum.as_str())
            && matches!(state, MeetingState::Processing | MeetingState::Completed);
        transaction.rollback().await?;
        Ok(if identical {
            FinalizeOutcome::Finalized(state)
        } else {
            FinalizeOutcome::Conflict
        })
    }

    pub async fn complete_owned_finalization(
        &self,
        user_id: &str,
        meeting_id: &str,
        last_sequence: i64,
        audio: &FinalAudioMetadata,
    ) -> anyhow::Result<FinalizeOutcome> {
        if !(0..=MAX_MEETING_CHUNK_SEQUENCE).contains(&last_sequence)
            || !safe_relative_path(&audio.relative_path)
            || audio.size_bytes < 0
            || audio.checksum.trim().is_empty()
        {
            bail!("invalid final meeting audio metadata");
        }
        if audio.relative_path != format!("{meeting_id}/recording.m4a") {
            bail!("invalid final meeting audio path");
        }
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let completed = sqlx::query(
            "UPDATE meetings
             SET state = 'processing',
                 duration_ms = (
                     SELECT COALESCE(MAX(end_ms), 0) FROM meeting_chunks
                     WHERE meeting_chunks.meeting_id = meetings.id AND sequence <= ?
                 ),
                 final_audio_path = ?, final_audio_size_bytes = ?,
                 final_audio_checksum = ?, error_stage = NULL, error_message = NULL,
                 updated_at = ?
             WHERE id = ? AND user_id = ? AND final_sequence = ? AND state = 'uploading'",
        )
        .bind(last_sequence)
        .bind(&audio.relative_path)
        .bind(audio.size_bytes)
        .bind(&audio.checksum)
        .bind(now)
        .bind(meeting_id)
        .bind(user_id)
        .bind(last_sequence)
        .execute(&mut *transaction)
        .await?;
        if completed.rows_affected() == 1 {
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
            return Ok(FinalizeOutcome::Finalized(MeetingState::Processing));
        }

        let row = sqlx::query(
            "SELECT state, final_sequence, final_audio_path,
                    final_audio_size_bytes, final_audio_checksum
             FROM meetings WHERE id = ? AND user_id = ?",
        )
        .bind(meeting_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            transaction.rollback().await?;
            return Ok(FinalizeOutcome::NotFound);
        };
        let state = MeetingState::parse(&row.get::<String, _>("state"))?;
        let has_audio = row.get::<Option<String>, _>("final_audio_path").is_some()
            && row
                .get::<Option<i64>, _>("final_audio_size_bytes")
                .is_some()
            && row
                .get::<Option<String>, _>("final_audio_checksum")
                .is_some();
        let outcome = if row.get::<Option<i64>, _>("final_sequence") == Some(last_sequence)
            && matches!(state, MeetingState::Processing | MeetingState::Completed)
            && has_audio
        {
            FinalizeOutcome::Finalized(state)
        } else {
            FinalizeOutcome::Conflict
        };
        transaction.rollback().await?;
        Ok(outcome)
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
        validate_transcript_segments(segments, false)?;
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

    pub async fn replace_provisional_segment(
        &self,
        meeting_id: &str,
        segment_id: i64,
        segment: Option<&TranscriptSegment>,
    ) -> anyhow::Result<()> {
        if segment_id < 0 {
            bail!("invalid provisional transcript segment id");
        }
        if let Some(segment) = segment
            && (segment.id != segment_id
                || !segment.provisional
                || segment.start_ms < 0
                || segment.end_ms <= segment.start_ms)
        {
            bail!("invalid provisional transcript segment");
        }
        if let Some(segment) = segment {
            sqlx::query(
                "INSERT INTO meeting_transcript_segments(
                    meeting_id, id, start_ms, end_ms, text, provisional
                 ) SELECT ?, ?, ?, ?, ?, 1
                   WHERE EXISTS (
                       SELECT 1 FROM meetings
                       WHERE id = ? AND state IN ('recording', 'paused', 'uploading', 'interrupted')
                   )
                 ON CONFLICT(meeting_id, id) DO UPDATE SET
                    start_ms = excluded.start_ms,
                    end_ms = excluded.end_ms,
                    text = excluded.text,
                    provisional = 1
                 WHERE meeting_transcript_segments.provisional = 1",
            )
            .bind(meeting_id)
            .bind(segment.id)
            .bind(segment.start_ms)
            .bind(segment.end_ms)
            .bind(&segment.text)
            .bind(meeting_id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM meeting_transcript_segments
                 WHERE meeting_id = ? AND id = ? AND provisional = 1",
            )
            .bind(meeting_id)
            .bind(segment_id)
            .execute(&self.pool)
            .await?;
        }
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

fn validate_transcript_segments(
    segments: &[TranscriptSegment],
    require_final: bool,
) -> anyhow::Result<()> {
    let mut ids = HashSet::new();
    for segment in segments {
        if segment.id < 0
            || segment.start_ms < 0
            || segment.end_ms <= segment.start_ms
            || (require_final && segment.provisional)
            || !ids.insert(segment.id)
        {
            bail!("invalid transcript segment timing or id");
        }
    }
    Ok(())
}

fn validate_meeting_artifact(artifact: &MeetingArtifact) -> anyhow::Result<()> {
    if artifact.title.trim().is_empty() || artifact.todos.len() > 50 {
        bail!("invalid meeting artifact title or todo count");
    }
    for todo in &artifact.todos {
        if todo.text.trim().is_empty() {
            bail!("meeting todo text must not be empty");
        }
        match (todo.source_start_ms, todo.source_end_ms) {
            (None, None) => {}
            (Some(start), Some(end)) if start >= 0 && end > start => {}
            _ => bail!("meeting todo source timing must be a valid complete range"),
        }
    }
    Ok(())
}

async fn claim_processing_job(
    pool: &SqlitePool,
    meeting_id: &str,
    stage: &str,
) -> anyhow::Result<TranscriptJobClaim> {
    let now = unix_time();
    let stale_before = now - 900.0;
    let claimed = sqlx::query(
        "UPDATE meeting_processing_jobs
         SET status = 'running', attempt = attempt + 1,
             diagnostic_error = NULL, updated_at = ?
         WHERE meeting_id = ? AND stage = ?
           AND (status IN ('pending', 'failed')
                OR (status = 'running' AND updated_at < ?))
         RETURNING attempt",
    )
    .bind(now)
    .bind(meeting_id)
    .bind(stage)
    .bind(stale_before)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = claimed {
        return Ok(TranscriptJobClaim::Claimed {
            attempt: row.get("attempt"),
        });
    }
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM meeting_processing_jobs WHERE meeting_id = ? AND stage = ?",
    )
    .bind(meeting_id)
    .bind(stage)
    .fetch_optional(pool)
    .await?;
    Ok(match status.as_deref() {
        Some("completed") => TranscriptJobClaim::Completed,
        Some("running") | Some("pending") | Some("failed") => TranscriptJobClaim::Busy,
        _ => TranscriptJobClaim::Missing,
    })
}

async fn heartbeat_processing_job(
    pool: &SqlitePool,
    meeting_id: &str,
    stage: &str,
    attempt: i64,
) -> anyhow::Result<bool> {
    let updated = sqlx::query(
        "UPDATE meeting_processing_jobs SET updated_at = ?
         WHERE meeting_id = ? AND stage = ? AND status = 'running' AND attempt = ?",
    )
    .bind(unix_time())
    .bind(meeting_id)
    .bind(stage)
    .bind(attempt)
    .execute(pool)
    .await?;
    Ok(updated.rows_affected() == 1)
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

    use sqlx::{Row, SqlitePool};
    use tokio::sync::Barrier;

    use crate::{
        context::ContextStore,
        meeting::types::{
            ChunkWrite, FinalAudioMetadata, FinalizeOutcome, MAX_MEETING_DURATION_MS,
            MeetingArtifact, MeetingState, MeetingTodo, MeetingTodoDraft, ProcessingStage,
            RetryStageOutcome, TranscriptJobClaim, TranscriptSegment,
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
    async fn initialize_preserves_and_closes_real_legacy_meetings() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let database_url = format!(
            "sqlite://{}?mode=rwc",
            directory.path().join("migration.sqlite3").display()
        );
        let pool = SqlitePool::connect(&database_url).await?;
        sqlx::query(
            "CREATE TABLE meetings (
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
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE meeting_chunks (
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
        )
        .execute(&pool)
        .await?;
        let recording_id = uuid::Uuid::new_v4().to_string();
        let processing_id = uuid::Uuid::new_v4().to_string();
        let completed_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO meetings(
                id, user_id, idempotency_key, state, started_at, created_at, updated_at
             ) VALUES (?, 'user-a', ?, 'recording', 100, 1, 1)",
        )
        .bind(&recording_id)
        .bind("legacy-recording")
        .execute(&pool)
        .await?;
        for (meeting_id, idempotency_key, state, checksum) in [
            (
                &processing_id,
                "legacy-processing",
                "processing",
                "a".repeat(64),
            ),
            (
                &completed_id,
                "legacy-completed",
                "completed",
                "b".repeat(64),
            ),
        ] {
            sqlx::query(
                "INSERT INTO meetings(
                    id, user_id, idempotency_key, state, started_at, ended_at, duration_ms,
                    final_audio_path, final_audio_size_bytes, final_audio_checksum,
                    created_at, updated_at
                 ) VALUES (?, 'user-a', ?, ?, 100, 110, 1000, ?, 123, ?, 1, 1)",
            )
            .bind(meeting_id)
            .bind(idempotency_key)
            .bind(state)
            .bind(format!("{meeting_id}/recording.m4a"))
            .bind(&checksum)
            .execute(&pool)
            .await?;
            sqlx::query(
                "INSERT INTO meeting_chunks(
                    meeting_id, sequence, start_ms, end_ms, checksum, size_bytes,
                    content_path, verified, created_at
                 ) VALUES (?, 0, 0, 1000, ?, 100, ?, 1, 1)",
            )
            .bind(meeting_id)
            .bind("c".repeat(64))
            .bind(format!("{meeting_id}/chunks/0.m4a"))
            .execute(&pool)
            .await?;
        }

        let store = MeetingStore::new(pool.clone());
        store.initialize().await?;

        let columns = sqlx::query("PRAGMA table_info(meetings)")
            .fetch_all(&pool)
            .await?;
        assert!(
            columns
                .iter()
                .any(|row| row.get::<String, _>("name") == "final_sequence")
        );
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings")
            .fetch_one(&pool)
            .await?;
        assert_eq!(rows, 3);
        assert_eq!(
            store
                .claim_finalization("user-a", &recording_id, 0, 101.0)
                .await?,
            FinalizeOutcome::Pending
        );
        let recording_boundary: Option<i64> =
            sqlx::query_scalar("SELECT final_sequence FROM meetings WHERE id = ?")
                .bind(&recording_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(recording_boundary, Some(0));

        for meeting_id in [&processing_id, &completed_id] {
            let expected_audio = store
                .owned_final_audio("user-a", meeting_id)
                .await?
                .expect("legacy final audio");
            assert_eq!(
                store
                    .claim_finalization("user-a", meeting_id, 0, 111.0)
                    .await?,
                FinalizeOutcome::LegacyVerificationRequired(expected_audio),
                "closed legacy meeting should request content verification"
            );
            let boundary: Option<i64> =
                sqlx::query_scalar("SELECT final_sequence FROM meetings WHERE id = ?")
                    .bind(meeting_id)
                    .fetch_one(&pool)
                    .await?;
            assert_eq!(boundary, None, "migration must not guess a boundary");
            assert_eq!(
                store
                    .record_verified_chunk(
                        meeting_id,
                        0,
                        0,
                        1000,
                        &"c".repeat(64),
                        100,
                        &format!("{meeting_id}/chunks/0.m4a"),
                    )
                    .await?,
                ChunkWrite::Existing
            );
            assert_eq!(
                store
                    .record_verified_chunk(
                        meeting_id,
                        1,
                        1000,
                        2000,
                        &"d".repeat(64),
                        100,
                        &format!("{meeting_id}/chunks/1.m4a"),
                    )
                    .await?,
                ChunkWrite::Conflict,
                "migration must not reopen the closed upload timeline"
            );
            let chunk_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM meeting_chunks WHERE meeting_id = ?")
                    .bind(meeting_id)
                    .fetch_one(&pool)
                    .await?;
            assert_eq!(chunk_count, 1, "closed timeline must not gain a row");
        }
        Ok(())
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
    async fn legacy_record_chunk_rejects_excessive_sequence() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store
            .create("user-a", "bounded-legacy-chunk", 100.0)
            .await?;

        let error = store
            .record_chunk(&meeting.id, 100_001, 0, 1_000, &"f".repeat(64), 100)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "meeting chunk sequence exceeds safety limit"
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_missing_sequence_query_rejects_excessive_boundary() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store
            .create("user-a", "bounded-legacy-finalization", 100.0)
            .await?;

        let error = store
            .missing_sequences(&meeting.id, 100_001)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "meeting final sequence exceeds safety limit"
        );
        Ok(())
    }

    #[tokio::test]
    async fn record_verified_chunk_rejects_excessive_sequence() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store.create("user-a", "bounded-chunk", 100.0).await?;

        let error = store
            .record_verified_chunk(
                &meeting.id,
                100_001,
                0,
                1_000,
                &"e".repeat(64),
                100,
                &format!("{}/chunks/100001.m4a", meeting.id),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "meeting chunk sequence exceeds safety limit"
        );
        Ok(())
    }

    #[tokio::test]
    async fn actual_audio_duration_is_capped_at_four_hours_without_counting_declared_gaps()
    -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let (meeting, _) = store
            .create_with_status("user-a", "four-hour-cap", 1.0)
            .await?;
        assert_eq!(
            store
                .record_verified_chunk(
                    &meeting.id,
                    0,
                    0,
                    MAX_MEETING_DURATION_MS,
                    "a",
                    1,
                    &format!("{}/chunks/0.m4a", meeting.id),
                )
                .await?,
            ChunkWrite::Inserted
        );
        assert_eq!(
            store
                .record_verified_chunk(
                    &meeting.id,
                    1,
                    MAX_MEETING_DURATION_MS,
                    MAX_MEETING_DURATION_MS + 1,
                    "b",
                    1,
                    &format!("{}/chunks/1.m4a", meeting.id),
                )
                .await?,
            ChunkWrite::DurationExceeded
        );

        let (gapped, _) = store.create_with_status("user-a", "large-gap", 1.0).await?;
        assert_eq!(
            store
                .record_verified_chunk(
                    &gapped.id,
                    0,
                    0,
                    100,
                    "c",
                    1,
                    &format!("{}/chunks/0.m4a", gapped.id),
                )
                .await?,
            ChunkWrite::Inserted
        );
        assert_eq!(
            store
                .record_verified_chunk(
                    &gapped.id,
                    1,
                    MAX_MEETING_DURATION_MS * 10,
                    MAX_MEETING_DURATION_MS * 10 + 100,
                    "d",
                    1,
                    &format!("{}/chunks/1.m4a", gapped.id),
                )
                .await?,
            ChunkWrite::Inserted
        );
        Ok(())
    }

    #[tokio::test]
    async fn finalization_rejects_legacy_audio_over_four_hours_before_creating_a_job()
    -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let (meeting, _) = store
            .create_with_status("user-a", "legacy-over-four-hours", 1.0)
            .await?;
        sqlx::query(
            "INSERT INTO meeting_chunks(
                meeting_id, sequence, start_ms, end_ms, checksum, size_bytes,
                content_path, verified, created_at
             ) VALUES (?, 0, 0, ?, 'legacy', 1, ?, 1, 1)",
        )
        .bind(&meeting.id)
        .bind(MAX_MEETING_DURATION_MS + 1)
        .bind(format!("{}/chunks/0.m4a", meeting.id))
        .execute(&context.pool_clone())
        .await?;

        assert_eq!(
            store
                .claim_finalization("user-a", &meeting.id, 0, 2.0)
                .await?,
            FinalizeOutcome::DurationExceeded
        );
        assert_eq!(
            store.get_owned("user-a", &meeting.id).await?.unwrap().state,
            MeetingState::Recording
        );
        let jobs = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM meeting_processing_jobs WHERE meeting_id = ?",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        assert_eq!(jobs, 0);
        Ok(())
    }

    #[tokio::test]
    async fn missing_sequence_query_rejects_excessive_boundary() -> anyhow::Result<()> {
        let (_directory, _context, store) = test_store().await;
        let meeting = store
            .create("user-a", "bounded-finalization", 100.0)
            .await?;

        let error = store
            .missing_verified_sequences(&meeting.id, 100_001)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "meeting final sequence exceeds safety limit"
        );
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
            .complete_owned_finalization("user-a", &meeting.id, 0, &audio)
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
                ChunkWrite::Conflict | ChunkWrite::DurationExceeded => {
                    anyhow::bail!("identical retry must not conflict")
                }
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

    #[tokio::test]
    async fn transcript_job_claim_is_single_flight_and_failed_jobs_retry() -> anyhow::Result<()> {
        use crate::meeting::types::TranscriptJobClaim;

        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "job-claim", 100.0).await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, 'transcript', 'pending', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;

        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Claimed { attempt: 1 }
        );
        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Busy
        );
        store.fail_transcript_job(&meeting.id, 1).await?;
        let failed: (String, i64, Option<String>) = sqlx::query_as(
            "SELECT status, attempt, diagnostic_error FROM meeting_processing_jobs
             WHERE meeting_id = ? AND stage = 'transcript'",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        let failed_meeting = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(
            failed,
            (
                "failed".to_owned(),
                1,
                Some("transcript processing failed".to_owned())
            )
        );
        assert_eq!(
            failed_meeting.error_stage,
            Some(ProcessingStage::Transcript)
        );
        assert_eq!(
            failed_meeting.error_message.as_deref(),
            Some("transcript processing failed")
        );
        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Claimed { attempt: 2 }
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_running_transcript_job_is_reclaimed_after_process_loss() -> anyhow::Result<()> {
        use crate::meeting::types::TranscriptJobClaim;

        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "job-reclaim", 100.0).await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, attempt, status, updated_at)
             VALUES (?, 'transcript', 1, 'running', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;

        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Claimed { attempt: 2 }
        );
        Ok(())
    }

    #[tokio::test]
    async fn transcript_job_completion_atomically_advances_organization() -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "job-complete", 100.0).await?;
        sqlx::query(
            "UPDATE meetings SET error_stage = 'transcript', error_message = 'safe' WHERE id = ?",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, attempt, status, updated_at)
             VALUES (?, 'transcript', 1, 'running', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;
        let segments = vec![TranscriptSegment {
            id: 0,
            start_ms: 500,
            end_ms: 1_000,
            text: "完成".to_owned(),
            provisional: false,
        }];

        store
            .complete_transcript_job(&meeting.id, 1, &segments)
            .await?;

        let transcript_status: String = sqlx::query_scalar(
            "SELECT status FROM meeting_processing_jobs WHERE meeting_id = ? AND stage = 'transcript'",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        let organization_status: String = sqlx::query_scalar(
            "SELECT status FROM meeting_processing_jobs WHERE meeting_id = ? AND stage = 'organization'",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(transcript_status, "completed");
        assert_eq!(organization_status, "pending");
        assert_eq!(detail.transcript, segments);
        assert_eq!(detail.error_stage, None);
        assert_eq!(detail.error_message, None);
        Ok(())
    }

    #[tokio::test]
    async fn stale_transcript_attempt_cannot_finish_a_reclaimed_job() -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "job-fence", 100.0).await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, attempt, status, updated_at)
             VALUES (?, 'transcript', 1, 'running', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;
        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Claimed { attempt: 2 }
        );

        store.fail_transcript_job(&meeting.id, 1).await?;
        assert!(
            store
                .complete_transcript_job(&meeting.id, 1, &[])
                .await
                .is_err()
        );
        let state: (String, i64) = sqlx::query_as(
            "SELECT status, attempt FROM meeting_processing_jobs
             WHERE meeting_id = ? AND stage = 'transcript'",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        assert_eq!(state, ("running".to_owned(), 2));
        Ok(())
    }

    #[tokio::test]
    async fn organization_completion_is_atomic_and_fenced_by_attempt() -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "organization-atomic", 100.0).await?;
        store
            .replace_artifact(
                &meeting.id,
                "旧标题",
                "旧摘要",
                &[MeetingTodo {
                    id: "old".to_owned(),
                    text: "旧待办".to_owned(),
                    completed: false,
                    source_start_ms: None,
                    source_end_ms: None,
                }],
            )
            .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, attempt, status, updated_at)
             VALUES (?, 'organization', 2, 'running', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;
        sqlx::query(
            "CREATE TRIGGER fail_meeting_todo_insert BEFORE INSERT ON meeting_todos
             BEGIN SELECT RAISE(ABORT, 'injected'); END",
        )
        .execute(&context.pool_clone())
        .await?;
        let artifact = MeetingArtifact {
            title: "新标题".to_owned(),
            summary: "新摘要".to_owned(),
            todos: vec![MeetingTodoDraft {
                text: "新待办".to_owned(),
                source_start_ms: None,
                source_end_ms: None,
            }],
        };

        assert!(
            store
                .complete_organization_job(&meeting.id, 2, &artifact)
                .await
                .is_err()
        );
        let unchanged = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(unchanged.title.as_deref(), Some("旧标题"));
        assert_eq!(unchanged.todos[0].text, "旧待办");
        let job: (String, i64) = sqlx::query_as(
            "SELECT status, attempt FROM meeting_processing_jobs
             WHERE meeting_id = ? AND stage = 'organization'",
        )
        .bind(&meeting.id)
        .fetch_one(&context.pool_clone())
        .await?;
        assert_eq!(job, ("running".to_owned(), 2));
        assert!(
            store
                .complete_organization_job(&meeting.id, 1, &artifact)
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn retry_stage_is_owned_idempotent_single_flight_and_recovers_stale_jobs()
    -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let meeting = store.create("user-a", "retry-stages", 100.0).await?;
        sqlx::query("UPDATE meetings SET final_audio_path = ?, final_audio_size_bytes = 1, final_audio_checksum = 'sum', state = 'processing' WHERE id = ?")
            .bind(format!("{}/recording.m4a", meeting.id))
            .bind(&meeting.id)
            .execute(&context.pool_clone())
            .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, attempt, status, updated_at)
             VALUES (?, 'transcript', 1, 'running', 1)",
        )
        .bind(&meeting.id)
        .execute(&context.pool_clone())
        .await?;

        assert_eq!(
            store
                .retry_stage_owned("user-b", &meeting.id, ProcessingStage::Transcript)
                .await?,
            RetryStageOutcome::NotFound
        );
        assert_eq!(
            store
                .retry_stage_owned("user-a", &meeting.id, ProcessingStage::Transcript)
                .await?,
            RetryStageOutcome::Queued
        );
        assert_eq!(
            store.claim_transcript_job(&meeting.id).await?,
            TranscriptJobClaim::Claimed { attempt: 2 }
        );
        assert_eq!(
            store
                .retry_stage_owned("user-a", &meeting.id, ProcessingStage::Transcript)
                .await?,
            RetryStageOutcome::Busy
        );
        store.complete_transcript_job(&meeting.id, 2, &[]).await?;
        assert_eq!(
            store
                .retry_stage_owned("user-a", &meeting.id, ProcessingStage::Transcript)
                .await?,
            RetryStageOutcome::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn completed_retry_is_idempotent_even_when_legacy_prerequisites_are_missing()
    -> anyhow::Result<()> {
        let (_directory, context, store) = test_store().await;
        let transcript = store
            .create("user-a", "completed-transcript", 100.0)
            .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, 'transcript', 'completed', 1)",
        )
        .bind(&transcript.id)
        .execute(&context.pool_clone())
        .await?;
        assert_eq!(
            store
                .retry_stage_owned("user-a", &transcript.id, ProcessingStage::Transcript)
                .await?,
            RetryStageOutcome::Completed
        );

        let organization = store
            .create("user-a", "completed-organization", 100.0)
            .await?;
        sqlx::query(
            "INSERT INTO meeting_processing_jobs(meeting_id, stage, status, updated_at)
             VALUES (?, 'organization', 'completed', 1)",
        )
        .bind(&organization.id)
        .execute(&context.pool_clone())
        .await?;
        assert_eq!(
            store
                .retry_stage_owned("user-a", &organization.id, ProcessingStage::Organization)
                .await?,
            RetryStageOutcome::Completed
        );
        Ok(())
    }
}
