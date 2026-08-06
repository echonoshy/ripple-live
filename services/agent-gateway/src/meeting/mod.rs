pub mod processor;
pub mod storage;
pub mod store;
pub mod types;

use sqlx::SqlitePool;

use storage::VerifiedLegacyFinalization;
use store::MeetingStore;
use types::{
    ChunkWrite, FinalAudioMetadata, FinalizeOutcome, Meeting, MeetingTodo, StoredChunkMetadata,
};

#[derive(Clone)]
pub struct MeetingService {
    store: MeetingStore,
}

impl MeetingService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            store: MeetingStore::new(pool),
        }
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        self.store.initialize().await
    }

    pub async fn create(
        &self,
        user_id: &str,
        idempotency_key: &str,
        started_at: f64,
    ) -> anyhow::Result<(Meeting, bool)> {
        self.store
            .create_with_status(user_id, idempotency_key, started_at)
            .await
    }

    pub async fn list(&self, user_id: &str) -> anyhow::Result<Vec<Meeting>> {
        self.store.list(user_id).await
    }

    pub async fn get_owned(
        &self,
        user_id: &str,
        meeting_id: &str,
    ) -> anyhow::Result<Option<Meeting>> {
        self.store.get_owned(user_id, meeting_id).await
    }

    pub async fn delete_owned(&self, user_id: &str, meeting_id: &str) -> anyhow::Result<bool> {
        self.store.delete_owned(user_id, meeting_id).await
    }

    pub async fn update_todo_completed(
        &self,
        user_id: &str,
        meeting_id: &str,
        todo_id: &str,
        completed: bool,
    ) -> anyhow::Result<Option<MeetingTodo>> {
        self.store
            .update_todo_completed(user_id, meeting_id, todo_id, completed)
            .await
    }

    #[allow(clippy::too_many_arguments)]
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
        self.store
            .record_verified_chunk(
                meeting_id,
                sequence,
                start_ms,
                end_ms,
                checksum,
                size_bytes,
                relative_path,
            )
            .await
    }

    pub async fn missing_verified_sequences(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<i64>> {
        self.store
            .missing_verified_sequences(meeting_id, last_sequence)
            .await
    }

    pub async fn verified_chunks(
        &self,
        meeting_id: &str,
        last_sequence: i64,
    ) -> anyhow::Result<Vec<StoredChunkMetadata>> {
        self.store.verified_chunks(meeting_id, last_sequence).await
    }

    pub async fn claim_finalization(
        &self,
        user_id: &str,
        meeting_id: &str,
        last_sequence: i64,
        ended_at: f64,
    ) -> anyhow::Result<FinalizeOutcome> {
        self.store
            .claim_finalization(user_id, meeting_id, last_sequence, ended_at)
            .await
    }

    pub async fn recover_legacy_finalization(
        &self,
        user_id: &str,
        proof: VerifiedLegacyFinalization,
    ) -> anyhow::Result<FinalizeOutcome> {
        self.store.recover_legacy_finalization(user_id, proof).await
    }

    pub async fn complete_owned_finalization(
        &self,
        user_id: &str,
        meeting_id: &str,
        last_sequence: i64,
        audio: &FinalAudioMetadata,
    ) -> anyhow::Result<FinalizeOutcome> {
        self.store
            .complete_owned_finalization(user_id, meeting_id, last_sequence, audio)
            .await
    }

    pub async fn owned_final_audio(
        &self,
        user_id: &str,
        meeting_id: &str,
    ) -> anyhow::Result<Option<FinalAudioMetadata>> {
        self.store.owned_final_audio(user_id, meeting_id).await
    }
}
