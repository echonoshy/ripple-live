use std::{path::Path, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{
    Acquire, QueryBuilder, Row, Sqlite, SqlitePool,
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
    pub is_pinned: bool,
    pub archived_at: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryScope {
    #[default]
    Active,
    Archived,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAction {
    Pin,
    Unpin,
    Archive,
    Unarchive,
    Delete,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: f64,
    pub attachments: Vec<ConversationAttachment>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationAttachment {
    pub id: String,
    pub kind: String,
    pub memory_id: Option<String>,
    pub caption: String,
    pub content_url: String,
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
                created_at REAL NOT NULL, updated_at REAL NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0, archived_at REAL
            )",
            "CREATE TABLE IF NOT EXISTS memory_items (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                conversation_id TEXT, source_turn_id INTEGER,
                source_response_id TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'visual',
                user_note TEXT NOT NULL, visual_summary TEXT NOT NULL DEFAULT '',
                cover_asset_id TEXT, captured_at REAL,
                created_at REAL NOT NULL, updated_at REAL NOT NULL,
                is_pinned INTEGER NOT NULL DEFAULT 0, archived_at REAL,
                FOREIGN KEY(user_id) REFERENCES users(id),
                FOREIGN KEY(conversation_id) REFERENCES conversations(id),
                FOREIGN KEY(source_turn_id) REFERENCES turns(id)
            )",
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, sha256 TEXT NOT NULL,
                mime_type TEXT NOT NULL, storage_key TEXT NOT NULL,
                width INTEGER NOT NULL, height INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL, captured_at REAL, created_at REAL NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id),
                UNIQUE(user_id, sha256)
            )",
            "CREATE TABLE IF NOT EXISTS memory_assets (
                memory_id TEXT NOT NULL, asset_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL, is_cover INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(memory_id, asset_id),
                FOREIGN KEY(memory_id) REFERENCES memory_items(id) ON DELETE CASCADE,
                FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE
            )",
            "CREATE TABLE IF NOT EXISTS turn_attachments (
                turn_id INTEGER NOT NULL, asset_id TEXT NOT NULL,
                memory_id TEXT, caption TEXT NOT NULL DEFAULT '',
                ordinal INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(turn_id, asset_id),
                FOREIGN KEY(turn_id) REFERENCES turns(id) ON DELETE CASCADE,
                FOREIGN KEY(asset_id) REFERENCES assets(id) ON DELETE CASCADE,
                FOREIGN KEY(memory_id) REFERENCES memory_items(id) ON DELETE SET NULL
            )",
            "CREATE TABLE IF NOT EXISTS memory_tool_executions (
                response_id TEXT NOT NULL, tool_call_id TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                PRIMARY KEY(response_id, tool_call_id),
                FOREIGN KEY(memory_id) REFERENCES memory_items(id) ON DELETE CASCADE
            )",
            "CREATE TABLE IF NOT EXISTS todos (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, memory_id TEXT,
                conversation_id TEXT, source_turn_id INTEGER, source_response_id TEXT,
                title TEXT NOT NULL, visual_summary TEXT NOT NULL DEFAULT '',
                cover_asset_id TEXT, due_at REAL, completed_at REAL,
                created_at REAL NOT NULL, updated_at REAL NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id),
                FOREIGN KEY(memory_id) REFERENCES memory_items(id) ON DELETE SET NULL,
                FOREIGN KEY(conversation_id) REFERENCES conversations(id),
                FOREIGN KEY(source_turn_id) REFERENCES turns(id),
                FOREIGN KEY(cover_asset_id) REFERENCES assets(id) ON DELETE SET NULL
            )",
            "CREATE TABLE IF NOT EXISTS todo_tool_executions (
                response_id TEXT NOT NULL, tool_call_id TEXT NOT NULL,
                todo_id TEXT NOT NULL,
                PRIMARY KEY(response_id, tool_call_id),
                FOREIGN KEY(todo_id) REFERENCES todos(id) ON DELETE CASCADE
            )",
            "CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id, id)",
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id, expires_at)",
            "CREATE INDEX IF NOT EXISTS idx_conversations_user ON conversations(user_id, updated_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_memory_items_user ON memory_items(user_id, created_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_memory_assets_memory ON memory_assets(memory_id, ordinal)",
            "CREATE INDEX IF NOT EXISTS idx_turn_attachments_turn ON turn_attachments(turn_id, ordinal)",
            "CREATE INDEX IF NOT EXISTS idx_todos_user_due ON todos(user_id, completed_at, due_at)",
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
        self.ensure_column("conversations", "is_pinned", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("conversations", "archived_at", "REAL")
            .await?;
        self.ensure_column("memory_items", "is_pinned", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("memory_items", "archived_at", "REAL")
            .await?;
        sqlx::query(
            "UPDATE conversations SET is_pinned = 0
             WHERE archived_at IS NOT NULL AND is_pinned != 0",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE memory_items SET is_pinned = 0
             WHERE archived_at IS NOT NULL AND is_pinned != 0",
        )
        .execute(&self.pool)
        .await?;
        self.ensure_column("todos", "conversation_id", "TEXT")
            .await?;
        self.ensure_column("todos", "source_turn_id", "INTEGER")
            .await?;
        self.ensure_column("todos", "source_response_id", "TEXT")
            .await?;
        self.ensure_column("todos", "visual_summary", "TEXT NOT NULL DEFAULT ''")
            .await?;
        self.ensure_column("todos", "cover_asset_id", "TEXT")
            .await?;
        self.migrate_memory_sources_nullable().await?;
        self.migrate_todo_evidence().await?;
        sqlx::query(
            "UPDATE invitation_codes SET use_count = 1
             WHERE used_by IS NOT NULL AND use_count = 0",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_todo_evidence(&self) -> anyhow::Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE todos
             SET visual_summary = COALESCE(NULLIF(visual_summary, ''),
                    (SELECT m.visual_summary FROM memory_items m WHERE m.id = todos.memory_id), ''),
                 cover_asset_id = COALESCE(cover_asset_id,
                    (SELECT m.cover_asset_id FROM memory_items m WHERE m.id = todos.memory_id)),
                 conversation_id = COALESCE(conversation_id,
                    (SELECT m.conversation_id FROM memory_items m WHERE m.id = todos.memory_id)),
                 source_turn_id = COALESCE(source_turn_id,
                    (SELECT m.source_turn_id FROM memory_items m WHERE m.id = todos.memory_id)),
                 source_response_id = COALESCE(source_response_id,
                    (SELECT m.source_response_id FROM memory_items m WHERE m.id = todos.memory_id))
             WHERE memory_id IS NOT NULL",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "DELETE FROM memory_items
             WHERE id IN (SELECT memory_id FROM todos WHERE memory_id IS NOT NULL)",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE todos SET memory_id = NULL WHERE memory_id IS NOT NULL")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn migrate_memory_sources_nullable(&self) -> anyhow::Result<()> {
        let columns = sqlx::query("PRAGMA table_info(memory_items)")
            .fetch_all(&self.pool)
            .await?;
        let requires_rebuild = columns.iter().any(|row| {
            row.get::<String, _>("name") == "conversation_id" && row.get::<i64, _>("notnull") != 0
        });
        if !requires_rebuild {
            return Ok(());
        }

        let mut connection = self.pool.acquire().await?;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await?;
        let migration = async {
            let mut transaction = connection.begin().await?;
            sqlx::query(
                "CREATE TABLE memory_items_new (
                    id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                    conversation_id TEXT, source_turn_id INTEGER,
                    source_response_id TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'visual',
                    user_note TEXT NOT NULL, visual_summary TEXT NOT NULL DEFAULT '',
                    cover_asset_id TEXT, captured_at REAL,
                    created_at REAL NOT NULL, updated_at REAL NOT NULL,
                    is_pinned INTEGER NOT NULL DEFAULT 0, archived_at REAL,
                    FOREIGN KEY(user_id) REFERENCES users(id),
                    FOREIGN KEY(conversation_id) REFERENCES conversations(id),
                    FOREIGN KEY(source_turn_id) REFERENCES turns(id)
                )",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO memory_items_new(
                    id, user_id, conversation_id, source_turn_id, source_response_id,
                    kind, user_note, visual_summary, cover_asset_id, captured_at,
                    created_at, updated_at, is_pinned, archived_at
                 ) SELECT id, user_id, conversation_id, source_turn_id, source_response_id,
                    kind, user_note, visual_summary, cover_asset_id, captured_at,
                    created_at, updated_at, is_pinned, archived_at
                 FROM memory_items",
            )
            .execute(&mut *transaction)
            .await?;
            sqlx::query("DROP TABLE memory_items")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("ALTER TABLE memory_items_new RENAME TO memory_items")
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_memory_items_user
                 ON memory_items(user_id, created_at DESC)",
            )
            .execute(&mut *transaction)
            .await?;
            let violations = sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(&mut *transaction)
                .await?;
            if !violations.is_empty() {
                anyhow::bail!("memory_items migration failed foreign key validation");
            }
            transaction.commit().await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await?;
        migration
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
        scope: LibraryScope,
        pinned_only: bool,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<ConversationSummary>> {
        let scope = match scope {
            LibraryScope::Active => 0_i64,
            LibraryScope::Archived => 1_i64,
            LibraryScope::All => 2_i64,
        };
        let query = query.trim();
        let pattern = format!("%{query}%");
        let rows = sqlx::query(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                c.is_pinned, c.archived_at,
                COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                          ORDER BY t.id DESC LIMIT 1), '') AS preview
             FROM conversations c WHERE c.user_id = ?
               AND (? = 2 OR (? = 0 AND c.archived_at IS NULL)
                    OR (? = 1 AND c.archived_at IS NOT NULL))
               AND (? = 0 OR c.is_pinned = 1)
               AND (? = '' OR c.title LIKE ? OR
                    COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                              ORDER BY t.id DESC LIMIT 1), '') LIKE ?)
             ORDER BY c.is_pinned DESC, c.updated_at DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(scope)
        .bind(scope)
        .bind(scope)
        .bind(i64::from(pinned_only))
        .bind(query)
        .bind(&pattern)
        .bind(&pattern)
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
                is_pinned: row.get::<i64, _>("is_pinned") != 0,
                archived_at: row.get("archived_at"),
            })
            .collect())
    }

    pub async fn conversation_summary(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<Option<ConversationSummary>> {
        let row = sqlx::query(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                c.is_pinned, c.archived_at,
                COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                          ORDER BY t.id DESC LIMIT 1), '') AS preview
             FROM conversations c WHERE c.user_id = ? AND c.id = ?",
        )
        .bind(user_id)
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ConversationSummary {
            id: row.get("id"),
            title: row.get("title"),
            preview: row.get("preview"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            is_pinned: row.get::<i64, _>("is_pinned") != 0,
            archived_at: row.get("archived_at"),
        }))
    }

    pub async fn rename_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
        title: &str,
    ) -> anyhow::Result<()> {
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("对话名称不能为空");
        }
        if title.chars().count() > 80 {
            anyhow::bail!("对话名称不能超过 80 个字符");
        }
        let result = sqlx::query(
            "UPDATE conversations SET title = ?, updated_at = ? WHERE id = ? AND user_id = ?",
        )
        .bind(title)
        .bind(unix_time())
        .bind(conversation_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            anyhow::bail!("对话不存在");
        }
        Ok(())
    }

    pub async fn mutate_conversations(
        &self,
        user_id: &str,
        ids: &[String],
        action: LibraryAction,
    ) -> anyhow::Result<usize> {
        validate_library_ids(ids)?;
        let mut transaction = self.pool.begin().await?;
        let mut ownership =
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM conversations WHERE user_id = ");
        ownership.push_bind(user_id).push(" AND id IN (");
        let mut separated = ownership.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        let count: i64 = ownership
            .build_query_scalar()
            .fetch_one(&mut *transaction)
            .await?;
        if count != ids.len() as i64 {
            anyhow::bail!("对话不存在");
        }

        if action == LibraryAction::Delete {
            let mut clear_sources = QueryBuilder::<Sqlite>::new(
                "UPDATE memory_items SET conversation_id = NULL, source_turn_id = NULL WHERE user_id = ",
            );
            clear_sources
                .push_bind(user_id)
                .push(" AND conversation_id IN (");
            let mut separated = clear_sources.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            clear_sources.build().execute(&mut *transaction).await?;

            for (prefix, nested) in [
                (
                    "DELETE FROM turn_attachments WHERE turn_id IN (SELECT id FROM turns WHERE session_id IN (",
                    true,
                ),
                ("DELETE FROM turns WHERE session_id IN (", false),
                ("DELETE FROM events WHERE session_id IN (", false),
                ("DELETE FROM memories WHERE session_id IN (", false),
                ("DELETE FROM sessions WHERE id IN (", false),
            ] {
                let mut delete = QueryBuilder::<Sqlite>::new(prefix);
                let mut separated = delete.separated(", ");
                for id in ids {
                    separated.push_bind(id);
                }
                separated.push_unseparated(if nested { "))" } else { ")" });
                delete.build().execute(&mut *transaction).await?;
            }
        } else {
            let mut update = QueryBuilder::<Sqlite>::new("UPDATE conversations SET ");
            match action {
                LibraryAction::Pin => update.push("is_pinned = CASE WHEN archived_at IS NULL THEN 1 ELSE 0 END"),
                LibraryAction::Unpin => update.push("is_pinned = 0"),
                LibraryAction::Archive => {
                    update.push("is_pinned = 0, archived_at = COALESCE(archived_at, ");
                    update.push_bind(unix_time()).push(")")
                }
                LibraryAction::Unarchive => update.push("archived_at = NULL"),
                LibraryAction::Delete => unreachable!(),
            };
            update.push(" WHERE user_id = ");
            update.push_bind(user_id).push(" AND id IN (");
            let mut separated = update.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            update.build().execute(&mut *transaction).await?;
        }

        if action == LibraryAction::Delete {
            let mut delete =
                QueryBuilder::<Sqlite>::new("DELETE FROM conversations WHERE user_id = ");
            delete.push_bind(user_id).push(" AND id IN (");
            let mut separated = delete.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            delete.build().execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(ids.len())
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
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let turn_id: i64 = row.get("id");
            let attachment_rows = sqlx::query(
                "SELECT a.id, ta.memory_id, ta.caption
                 FROM turn_attachments ta JOIN assets a ON a.id = ta.asset_id
                 WHERE ta.turn_id = ? AND a.user_id = ? ORDER BY ta.ordinal ASC",
            )
            .bind(turn_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            messages.push(ConversationMessage {
                id: turn_id,
                role: row.get("role"),
                content: row.get("content"),
                created_at: row.get("created_at"),
                attachments: attachment_rows
                    .into_iter()
                    .map(|attachment| {
                        let id: String = attachment.get("id");
                        ConversationAttachment {
                            content_url: format!("/v1/assets/{id}/content"),
                            id,
                            kind: "image".to_owned(),
                            memory_id: attachment.get("memory_id"),
                            caption: attachment.get("caption"),
                        }
                    })
                    .collect(),
            });
        }
        Ok(Some(messages))
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
    ) -> anyhow::Result<i64> {
        let now = unix_time();
        let result = sqlx::query(
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
        Ok(result.last_insert_rowid())
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
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

    pub async fn trailing_user_input(
        &self,
        session_id: &str,
        max_turns: i64,
    ) -> anyhow::Result<(String, usize)> {
        let rows = sqlx::query(
            "SELECT role, content FROM turns WHERE session_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(max_turns.clamp(1, 8))
        .fetch_all(&self.pool)
        .await?;

        let mut fragments = Vec::new();
        for row in rows {
            let role: String = row.get("role");
            if role != "user" {
                break;
            }
            let content: String = row.get("content");
            if !content.trim().is_empty() {
                fragments.push(content);
            }
        }
        fragments.reverse();
        let count = fragments.len();
        Ok((fragments.join(" "), count))
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

fn validate_library_ids(ids: &[String]) -> anyhow::Result<()> {
    if ids.is_empty() || ids.len() > 100 {
        anyhow::bail!("ids 必须包含 1 到 100 个项目");
    }
    let unique = ids.iter().collect::<std::collections::HashSet<_>>();
    if unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty()) {
        anyhow::bail!("ids 不能包含空值或重复项");
    }
    Ok(())
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

    async fn registered_user(store: &ContextStore, email: &str, invite: &str) -> AuthUser {
        store
            .seed_invitation_codes(&[invite.to_owned()], 1, 24)
            .await
            .unwrap();
        store
            .register_user(email, "password-library", invite, 24)
            .await
            .unwrap()
            .0
    }

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
    async fn joins_unanswered_user_fragments_for_routing() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("routing.sqlite3"))
            .await
            .unwrap();
        store.touch_session("s1").await.unwrap();
        store
            .add_turn("s1", "user", "帮我搜索一下", None)
            .await
            .unwrap();
        store
            .add_turn("s1", "user", "记忆图库里的", None)
            .await
            .unwrap();
        store
            .add_turn("s1", "user", "芦荟胶图片", None)
            .await
            .unwrap();
        let (input, turns) = store.trailing_user_input("s1", 4).await.unwrap();
        assert_eq!(turns, 3);
        assert_eq!(input, "帮我搜索一下 记忆图库里的 芦荟胶图片");

        store
            .add_turn("s1", "assistant", "好的", None)
            .await
            .unwrap();
        store
            .add_turn("s1", "user", "查看待办", None)
            .await
            .unwrap();
        let (input, turns) = store.trailing_user_input("s1", 4).await.unwrap();
        assert_eq!(turns, 1);
        assert_eq!(input, "查看待办");
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

    #[tokio::test]
    async fn organizes_conversations_and_rejects_non_owned_batches_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("library.sqlite3"))
            .await
            .unwrap();
        let user = registered_user(&store, "library@example.com", "library-invite").await;
        let other = registered_user(&store, "other-library@example.com", "other-invite").await;
        let conversation = store.create_conversation(&user.id).await.unwrap();
        let other_conversation = store.create_conversation(&other.id).await.unwrap();
        store
            .add_turn(&conversation, "user", "蓝色转接头放在哪里", None)
            .await
            .unwrap();

        let active = store
            .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
            .await
            .unwrap();
        assert!(!active[0].is_pinned);
        assert_eq!(active[0].archived_at, None);
        assert_eq!(
            store
                .list_conversations(&user.id, LibraryScope::Active, false, "转接头", 50)
                .await
                .unwrap()
                .len(),
            1
        );

        store
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation),
                LibraryAction::Pin,
            )
            .await
            .unwrap();
        assert!(
            store
                .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
                .await
                .unwrap()[0]
                .is_pinned
        );

        let mixed = vec![conversation.clone(), other_conversation];
        assert!(
            store
                .mutate_conversations(&user.id, &mixed, LibraryAction::Archive)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
                .await
                .unwrap()
                .len(),
            1
        );

        store
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation),
                LibraryAction::Archive,
            )
            .await
            .unwrap();
        assert!(
            store
                .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .list_conversations(&user.id, LibraryScope::Archived, false, "", 50)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn migrates_legacy_memory_sources_to_nullable_without_losing_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.sqlite3");
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE users (
                id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL, created_at REAL NOT NULL
            )",
            "CREATE TABLE conversations (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '新对话',
                created_at REAL NOT NULL, updated_at REAL NOT NULL
            )",
            "CREATE TABLE turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
                role TEXT NOT NULL, content TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}', created_at REAL NOT NULL
            )",
            "CREATE TABLE memory_items (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL, source_turn_id INTEGER NOT NULL,
                source_response_id TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'visual',
                user_note TEXT NOT NULL, visual_summary TEXT NOT NULL DEFAULT '',
                cover_asset_id TEXT, captured_at REAL,
                created_at REAL NOT NULL, updated_at REAL NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id),
                FOREIGN KEY(conversation_id) REFERENCES conversations(id),
                FOREIGN KEY(source_turn_id) REFERENCES turns(id)
            )",
            "INSERT INTO users VALUES ('user-1', 'legacy@example.com', 'hash', 1)",
            "INSERT INTO conversations VALUES ('conv-1', 'user-1', '旧对话', 1, 1)",
            "INSERT INTO turns(id, session_id, role, content, created_at)
             VALUES (1, 'conv-1', 'user', '旧消息', 1)",
            "INSERT INTO memory_items(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                user_note, created_at, updated_at
             ) VALUES ('mem-1', 'user-1', 'conv-1', 1, 'response-1', '旧记忆', 1, 1)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool.close().await;

        let store = ContextStore::open(&path).await.unwrap();
        let columns = sqlx::query("PRAGMA table_info(memory_items)")
            .fetch_all(store.pool())
            .await
            .unwrap();
        for source in ["conversation_id", "source_turn_id"] {
            let column = columns
                .iter()
                .find(|row| row.get::<String, _>("name") == source)
                .unwrap();
            assert_eq!(column.get::<i64, _>("notnull"), 0);
        }
        let row = sqlx::query(
            "SELECT conversation_id, source_turn_id, is_pinned, archived_at
             FROM memory_items WHERE id = 'mem-1'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(
            row.get::<Option<String>, _>("conversation_id").as_deref(),
            Some("conv-1")
        );
        assert_eq!(row.get::<Option<i64>, _>("source_turn_id"), Some(1));
        assert_eq!(row.get::<i64, _>("is_pinned"), 0);
        assert_eq!(row.get::<Option<f64>, _>("archived_at"), None);
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(store.pool())
                .await
                .unwrap()
                .is_empty()
        );
    }
}
