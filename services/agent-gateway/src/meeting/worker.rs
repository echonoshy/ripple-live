use std::time::Duration;

use anyhow::bail;
use tokio::task::JoinHandle;

use super::{processor::MeetingProcessor, store::MeetingStore, types::ProcessingStage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMeetingJob {
    pub meeting_id: String,
    pub stage: ProcessingStage,
    pub audio_path: String,
}

#[derive(Clone)]
pub struct MeetingWorker {
    store: MeetingStore,
    processor: MeetingProcessor,
    batch_size: i64,
}

impl MeetingWorker {
    pub fn new(
        store: MeetingStore,
        processor: MeetingProcessor,
        batch_size: i64,
    ) -> anyhow::Result<Self> {
        if batch_size <= 0 {
            bail!("meeting worker batch size must be positive");
        }
        Ok(Self {
            store,
            processor,
            batch_size,
        })
    }

    pub async fn run_once(&self) -> anyhow::Result<usize> {
        let jobs = self.store.runnable_processing_jobs(self.batch_size).await?;
        for job in &jobs {
            self.processor.process_job(job.clone()).await;
        }
        Ok(jobs.len())
    }

    pub fn spawn(&self, interval: Duration) -> JoinHandle<()> {
        let worker = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = worker.run_once().await {
                    tracing::warn!(
                        error = %format!("{error:#}"),
                        "meeting worker iteration failed"
                    );
                }
                tokio::time::sleep(interval).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use sha2::{Digest, Sha256};
    use tokio::process::Command;

    use crate::{
        adapters::ModelAdapters,
        config::Settings,
        context::ContextStore,
        meeting::{
            processor::MeetingProcessor, storage::MeetingStorage, store::MeetingStore,
            types::ProcessingJobStatus,
        },
    };

    use super::MeetingWorker;

    async fn generate_m4a(path: &Path) -> anyhow::Result<()> {
        let status = Command::new("/usr/bin/ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.36",
                "-c:a",
                "aac",
            ])
            .arg(path)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "fixture generation failed");
        Ok(())
    }

    #[tokio::test]
    async fn worker_reclaims_stale_transcript_after_restart() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let meeting = store.create("user-a", "restart", 1.0).await?;
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            "/usr/bin/ffmpeg".into(),
        );
        let relative_path = format!("{}/recording.m4a", meeting.id);
        let absolute_path = directory.path().join("audio").join(&relative_path);
        tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
        generate_m4a(&absolute_path).await?;
        let bytes = tokio::fs::read(&absolute_path).await?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        store
            .record_verified_chunk(
                &meeting.id,
                0,
                0,
                360,
                &checksum,
                bytes.len() as i64,
                &format!("{}/chunks/0.m4a", meeting.id),
            )
            .await?;
        sqlx::query(
            "UPDATE meetings
             SET state = 'processing', final_audio_path = ?, final_audio_size_bytes = ?,
                 final_audio_checksum = ? WHERE id = ?",
        )
        .bind(&relative_path)
        .bind(bytes.len() as i64)
        .bind(&checksum)
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
        let (adapters, _) = ModelAdapters::with_transcription_test_results(
            Settings::from_env()?,
            vec![Ok("恢复后的逐字稿".to_owned())],
            Duration::ZERO,
        )?;
        let processor = MeetingProcessor::new(store.clone(), storage, adapters, 1)?;
        let worker = MeetingWorker::new(store.clone(), processor, 10)?;

        assert_eq!(worker.run_once().await?, 1);
        assert_eq!(
            store
                .processing_progress(&meeting.id)
                .await?
                .transcript
                .status,
            ProcessingJobStatus::Completed
        );
        Ok(())
    }
}
