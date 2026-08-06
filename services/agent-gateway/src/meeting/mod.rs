pub mod store;
pub mod types;

use sqlx::SqlitePool;

use store::MeetingStore;
use types::{Meeting, MeetingTodo};

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
}
