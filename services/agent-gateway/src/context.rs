use std::{path::Path, str::FromStr};

use serde_json::Value;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

#[derive(Clone)]
pub struct ContextStore {
    pool: SqlitePool,
}

impl ContextStore {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.initialize().await?;
        Ok(store)
    }

    async fn initialize(&self) -> anyhow::Result<()> {
        for statement in [
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY, created_at REAL NOT NULL, updated_at REAL NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
                kind TEXT NOT NULL, payload TEXT NOT NULL, created_at REAL NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
                role TEXT NOT NULL, content TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}', created_at REAL NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
                content TEXT NOT NULL, created_at REAL NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id, id)",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn touch_session(&self, session_id: &str) -> anyhow::Result<()> {
        let now = unix_time();
        sqlx::query(
            "INSERT INTO sessions(id, created_at, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET updated_at = excluded.updated_at",
        )
        .bind(session_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_event(
        &self,
        session_id: &str,
        kind: &str,
        payload: &Value,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO events(session_id, kind, payload, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(kind)
        .bind(serde_json::to_string(payload)?)
        .bind(unix_time())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&Value>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO turns(session_id, role, content, metadata, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(role)
        .bind(content)
        .bind(serde_json::to_string(
            metadata.unwrap_or(&Value::Object(Default::default())),
        )?)
        .bind(unix_time())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn recent_messages(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT role, content FROM turns WHERE session_id = ?
             ORDER BY id DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(limit * 2)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .rev()
            .map(|row| {
                serde_json::json!({
                    "role": row.get::<String, _>("role"),
                    "content": row.get::<String, _>("content")
                })
            })
            .collect())
    }

    pub async fn remember(&self, session_id: &str, content: &str) -> anyhow::Result<i64> {
        let result =
            sqlx::query("INSERT INTO memories(session_id, content, created_at) VALUES (?, ?, ?)")
                .bind(session_id)
                .bind(content)
                .bind(unix_time())
                .execute(&self.pool)
                .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn recall(
        &self,
        session_id: &str,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<String>> {
        let rows = if query.is_empty() {
            sqlx::query(
                "SELECT content FROM memories WHERE session_id = ?
                 ORDER BY id DESC LIMIT ?",
            )
            .bind(session_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT content FROM memories WHERE session_id = ? AND content LIKE ?
                 ORDER BY id DESC LIMIT ?",
            )
            .bind(session_id)
            .bind(format!("%{query}%"))
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("content"))
            .collect())
    }
}

fn unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stores_and_recalls_memory() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("test.sqlite3"))
            .await
            .unwrap();
        store.touch_session("s1").await.unwrap();
        store.remember("s1", "用户喜欢乌龙茶").await.unwrap();
        assert_eq!(
            store.recall("s1", "乌龙", 5).await.unwrap(),
            vec!["用户喜欢乌龙茶"]
        );
    }
}
