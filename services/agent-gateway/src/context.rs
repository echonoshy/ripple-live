use std::{path::Path, str::FromStr};

use serde::Serialize;
use serde_json::Value;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use uuid::Uuid;

use crate::auth::{
    AuthUser, hash_password, new_access_token, normalize_email, secret_hash, verify_password,
};

#[derive(Clone, Debug, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: f64,
}

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
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL, created_at REAL NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS invitation_codes (
                code_hash TEXT PRIMARY KEY, created_at REAL NOT NULL,
                used_by TEXT, used_at REAL,
                max_uses INTEGER NOT NULL DEFAULT 1,
                use_count INTEGER NOT NULL DEFAULT 0,
                expires_at REAL
            )",
            "CREATE TABLE IF NOT EXISTS invitation_redemptions (
                code_hash TEXT NOT NULL, user_id TEXT NOT NULL, used_at REAL NOT NULL,
                PRIMARY KEY(code_hash, user_id)
            )",
            "CREATE TABLE IF NOT EXISTS auth_sessions (
                token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                created_at REAL NOT NULL, expires_at REAL NOT NULL,
                revoked_at REAL
            )",
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '新对话',
                created_at REAL NOT NULL, updated_at REAL NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id, expires_at)",
            "CREATE INDEX IF NOT EXISTS idx_conversations_user ON conversations(user_id, updated_at DESC)",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        self.ensure_column("invitation_codes", "max_uses", "INTEGER NOT NULL DEFAULT 1")
            .await?;
        self.ensure_column(
            "invitation_codes",
            "use_count",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column("invitation_codes", "expires_at", "REAL")
            .await?;
        sqlx::query(
            "UPDATE invitation_codes SET use_count = 1
             WHERE used_by IS NOT NULL AND use_count = 0",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn ensure_column(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> anyhow::Result<()> {
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&self.pool)
            .await?;
        if rows
            .iter()
            .any(|row| row.get::<String, _>("name") == column)
        {
            return Ok(());
        }
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn seed_invitation_codes(
        &self,
        codes: &[String],
        max_uses: i64,
        ttl_hours: i64,
    ) -> anyhow::Result<()> {
        let now = unix_time();
        let max_uses = max_uses.clamp(1, 1_000_000);
        let expires_at = now + ttl_hours.clamp(0, 24 * 365 * 10) as f64 * 3600.0;
        for code in codes {
            sqlx::query(
                "INSERT INTO invitation_codes(
                    code_hash, created_at, max_uses, use_count, expires_at
                 ) VALUES (?, ?, ?, 0, ?)
                 ON CONFLICT(code_hash) DO UPDATE SET
                    max_uses = excluded.max_uses,
                    expires_at = COALESCE(invitation_codes.expires_at, excluded.expires_at)",
            )
            .bind(secret_hash(code))
            .bind(now)
            .bind(max_uses)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn register_user(
        &self,
        email: &str,
        password: &str,
        invitation_code: &str,
        token_ttl_hours: i64,
    ) -> anyhow::Result<(AuthUser, String)> {
        let email = normalize_email(email)?;
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password)).await??;
        let invitation_hash = secret_hash(invitation_code.trim());
        let now = unix_time();
        let user_id = Uuid::new_v4().to_string();
        let mut transaction = self.pool.begin().await?;

        let available = sqlx::query(
            "SELECT 1 FROM invitation_codes
             WHERE code_hash = ? AND use_count < max_uses AND expires_at > ?",
        )
        .bind(&invitation_hash)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some();
        if !available {
            anyhow::bail!("邀请码无效、已达使用上限或已经过期");
        }
        let exists = sqlx::query("SELECT 1 FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some();
        if exists {
            anyhow::bail!("该邮箱已经注册");
        }
        sqlx::query("INSERT INTO users(id, email, password_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(&user_id)
            .bind(&email)
            .bind(password_hash)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        let consumed = sqlx::query(
            "UPDATE invitation_codes
             SET used_by = ?, used_at = ?, use_count = use_count + 1
             WHERE code_hash = ? AND use_count < max_uses AND expires_at > ?",
        )
        .bind(&user_id)
        .bind(now)
        .bind(invitation_hash)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if consumed.rows_affected() != 1 {
            anyhow::bail!("邀请码已达使用上限或已经过期");
        }
        sqlx::query(
            "INSERT INTO invitation_redemptions(code_hash, user_id, used_at)
             VALUES (?, ?, ?)",
        )
        .bind(secret_hash(invitation_code.trim()))
        .bind(&user_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let token = insert_auth_session(&mut transaction, &user_id, now, token_ttl_hours).await?;
        transaction.commit().await?;
        Ok((AuthUser { id: user_id, email }, token))
    }

    pub async fn login_user(
        &self,
        email: &str,
        password: &str,
        token_ttl_hours: i64,
    ) -> anyhow::Result<(AuthUser, String)> {
        let email = normalize_email(email)?;
        let row = sqlx::query("SELECT id, password_hash FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("邮箱或密码错误"))?;
        let password_hash = row.get::<String, _>("password_hash");
        let password = password.to_owned();
        let verified =
            tokio::task::spawn_blocking(move || verify_password(&password, &password_hash)).await?;
        if !verified {
            anyhow::bail!("邮箱或密码错误");
        }
        let user_id = row.get::<String, _>("id");
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        let token = insert_auth_session(&mut transaction, &user_id, now, token_ttl_hours).await?;
        transaction.commit().await?;
        Ok((AuthUser { id: user_id, email }, token))
    }

    pub async fn authenticate(&self, token: &str) -> anyhow::Result<Option<AuthUser>> {
        let row = sqlx::query(
            "SELECT users.id, users.email FROM auth_sessions
             JOIN users ON users.id = auth_sessions.user_id
             WHERE auth_sessions.token_hash = ? AND auth_sessions.revoked_at IS NULL
             AND auth_sessions.expires_at > ?",
        )
        .bind(secret_hash(token))
        .bind(unix_time())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| AuthUser {
            id: row.get("id"),
            email: row.get("email"),
        }))
    }

    pub async fn revoke_token(&self, token: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE auth_sessions SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(unix_time())
        .bind(secret_hash(token))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_conversation(&self, user_id: &str) -> anyhow::Result<String> {
        let id = format!("conv_{}", Uuid::new_v4().simple());
        let now = unix_time();
        sqlx::query(
            "INSERT INTO conversations(id, user_id, title, created_at, updated_at)
             VALUES (?, ?, '新对话', ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.touch_session(&id).await?;
        Ok(id)
    }

    pub async fn conversation_belongs_to(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM conversations WHERE id = ? AND user_id = ?")
                .bind(conversation_id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?
                .is_some(),
        )
    }

    pub async fn list_conversations(
        &self,
        user_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<ConversationSummary>> {
        let rows = sqlx::query(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                          ORDER BY t.id DESC LIMIT 1), '') AS preview
             FROM conversations c WHERE c.user_id = ?
             ORDER BY c.updated_at DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ConversationSummary {
                id: row.get("id"),
                title: row.get("title"),
                preview: row.get("preview"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn conversation_messages(
        &self,
        user_id: &str,
        conversation_id: &str,
        limit: i64,
    ) -> anyhow::Result<Option<Vec<ConversationMessage>>> {
        if !self
            .conversation_belongs_to(conversation_id, user_id)
            .await?
        {
            return Ok(None);
        }
        let rows = sqlx::query(
            "SELECT id, role, content, created_at FROM turns WHERE session_id = ?
             ORDER BY id ASC LIMIT ?",
        )
        .bind(conversation_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(
            rows.into_iter()
                .map(|row| ConversationMessage {
                    id: row.get("id"),
                    role: row.get("role"),
                    content: row.get("content"),
                    created_at: row.get("created_at"),
                })
                .collect(),
        ))
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
        let now = unix_time();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO events(session_id, kind, payload, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(kind)
        .bind(serde_json::to_string(payload)?)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn add_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&Value>,
    ) -> anyhow::Result<()> {
        let now = unix_time();
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
        .bind(now)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE conversations SET updated_at = ?,
             title = CASE WHEN title = '新对话' AND ? = 'user'
                          THEN substr(?, 1, 32) ELSE title END
             WHERE id = ?",
        )
        .bind(now)
        .bind(role)
        .bind(content.trim())
        .bind(session_id)
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

    pub async fn recent_memories(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT content FROM memories WHERE session_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("content"))
            .collect())
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

async fn insert_auth_session(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    now: f64,
    token_ttl_hours: i64,
) -> anyhow::Result<String> {
    let token = new_access_token();
    let expires_at = now + token_ttl_hours.clamp(1, 24 * 365) as f64 * 3600.0;
    sqlx::query(
        "INSERT INTO auth_sessions(token_hash, user_id, created_at, expires_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(secret_hash(&token))
    .bind(user_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut **transaction)
    .await?;
    Ok(token)
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

    #[tokio::test]
    async fn enforces_invite_limits_expiration_and_conversation_isolation() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("auth.sqlite3"))
            .await
            .unwrap();
        store
            .seed_invitation_codes(&["invite-one".to_owned(), "invite-two".to_owned()], 2, 24)
            .await
            .unwrap();
        let (first, token) = store
            .register_user("FIRST@example.com", "password-one", "invite-one", 24)
            .await
            .unwrap();
        assert_eq!(first.email, "first@example.com");
        assert_eq!(
            store.authenticate(&token).await.unwrap(),
            Some(first.clone())
        );
        store
            .register_user("other@example.com", "password-two", "invite-one", 24)
            .await
            .unwrap();
        assert!(
            store
                .register_user("third@example.com", "password-three", "invite-one", 24)
                .await
                .is_err()
        );
        let (second, _) = store
            .register_user("second@example.com", "password-two", "invite-two", 24)
            .await
            .unwrap();
        let conversation = store.create_conversation(&first.id).await.unwrap();
        store
            .add_turn(&conversation, "user", "今天聊认证系统", None)
            .await
            .unwrap();
        assert!(
            store
                .conversation_messages(&first.id, &conversation, 20)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .conversation_messages(&second.id, &conversation, 20)
                .await
                .unwrap()
                .is_none()
        );
        store.revoke_token(&token).await.unwrap();
        assert!(store.authenticate(&token).await.unwrap().is_none());

        let expired_store = ContextStore::open(&directory.path().join("expired.sqlite3"))
            .await
            .unwrap();
        expired_store
            .seed_invitation_codes(&["expired-invite".to_owned()], 5, 0)
            .await
            .unwrap();
        assert!(
            expired_store
                .register_user(
                    "expired@example.com",
                    "password-expired",
                    "expired-invite",
                    24,
                )
                .await
                .is_err()
        );
    }
}
