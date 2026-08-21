use std::{path::Path, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{
    PgPool, Postgres, QueryBuilder, Row,
    postgres::{PgConnectOptions, PgPoolOptions},
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

#[derive(Clone, Debug, Serialize)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub archived_at: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UserProfile {
    pub ai_identity: String,
    pub user_identity: String,
    pub preferred_name: String,
    pub basic_memory: String,
    pub updated_at: Option<f64>,
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
    pub actions: Vec<ConversationAction>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationAction {
    pub kind: String,
    pub target_id: String,
    pub label: String,
    pub due_at: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConversationAttachment {
    pub id: String,
    pub kind: String,
    pub memory_id: Option<String>,
    pub todo_id: Option<String>,
    pub caption: String,
    pub content_url: String,
}

#[derive(Clone)]
pub struct ContextStore {
    pool: PgPool,
}

impl ContextStore {
    pub async fn connect(database_url: &str, max_connections: u32) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections.max(1))
            .connect(database_url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    #[doc(hidden)]
    pub async fn open(_path: &Path) -> anyhow::Result<Self> {
        let database_url = std::env::var("RIPPLE_TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@127.0.0.1:55432/ripple_test".to_owned()
        });
        let schema = format!("test_{}", Uuid::new_v4().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await?;
        sqlx::query("SELECT pg_advisory_lock(727105001)")
            .execute(&admin)
            .await?;
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&admin)
            .await?;
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_trgm")
            .execute(&admin)
            .await?;
        sqlx::query("SELECT pg_advisory_unlock(727105001)")
            .execute(&admin)
            .await?;
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await?;
        admin.close().await;

        let search_path = format!("{schema},public");
        let options = PgConnectOptions::from_str(&database_url)?
            .options([("search_path", search_path.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
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
                 ) VALUES ($1, $2, $3, 0, $4)
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
             WHERE code_hash = $1 AND use_count < max_uses AND expires_at > $2",
        )
        .bind(&invitation_hash)
        .bind(now)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some();
        if !available {
            anyhow::bail!("邀请码无效、已达使用上限或已经过期");
        }
        let exists = sqlx::query("SELECT 1 FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some();
        if exists {
            anyhow::bail!("该邮箱已经注册");
        }
        sqlx::query(
            "INSERT INTO users(id, email, password_hash, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(&user_id)
        .bind(&email)
        .bind(password_hash)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let consumed = sqlx::query(
            "UPDATE invitation_codes
             SET used_by = $1, used_at = $2, use_count = use_count + 1
             WHERE code_hash = $3 AND use_count < max_uses AND expires_at > $4",
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
             VALUES ($1, $2, $3)",
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
        let row = sqlx::query("SELECT id, password_hash FROM users WHERE email = $1")
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
             WHERE auth_sessions.token_hash = $1 AND auth_sessions.revoked_at IS NULL
             AND auth_sessions.expires_at > $2",
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
            "UPDATE auth_sessions SET revoked_at = $1 WHERE token_hash = $2 AND revoked_at IS NULL",
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
             VALUES ($1, $2, '新对话', $3, $4)",
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

    pub async fn user_profile(&self, user_id: &str) -> anyhow::Result<UserProfile> {
        let row = sqlx::query(
            "SELECT ai_identity, user_identity, preferred_name, basic_memory, updated_at
             FROM user_profiles WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_profile_from_row).unwrap_or_default())
    }

    pub async fn update_user_profile(
        &self,
        user_id: &str,
        ai_identity: &str,
        user_identity: &str,
        preferred_name: &str,
        basic_memory: &str,
    ) -> anyhow::Result<UserProfile> {
        validate_user_profile(ai_identity, user_identity, preferred_name, basic_memory)?;
        let now = unix_time();
        sqlx::query(
            "INSERT INTO user_profiles(
                user_id, ai_identity, user_identity, preferred_name, basic_memory, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(user_id) DO UPDATE SET
                ai_identity = EXCLUDED.ai_identity,
                user_identity = EXCLUDED.user_identity,
                preferred_name = EXCLUDED.preferred_name,
                basic_memory = EXCLUDED.basic_memory,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(user_id)
        .bind(ai_identity.trim())
        .bind(user_identity.trim())
        .bind(preferred_name.trim())
        .bind(basic_memory.trim())
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.user_profile(user_id).await
    }

    pub async fn create_project(
        &self,
        user_id: &str,
        name: &str,
        description: &str,
        instructions: &str,
    ) -> anyhow::Result<ProjectRecord> {
        validate_project_fields(name, description, instructions)?;
        let id = format!("proj_{}", Uuid::new_v4().simple());
        let now = unix_time();
        sqlx::query(
            "INSERT INTO projects(
                id, owner_user_id, name, description, instructions,
                status, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, 'active', $6, $7)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(name.trim())
        .bind(description.trim())
        .bind(instructions.trim())
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.project(user_id, &id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的项目不存在"))
    }

    pub async fn project(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> anyhow::Result<Option<ProjectRecord>> {
        let row = sqlx::query(
            "SELECT id, name, description, instructions, created_at, updated_at, archived_at
             FROM projects WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(project_from_row))
    }

    pub async fn list_projects(
        &self,
        user_id: &str,
        scope: LibraryScope,
        limit: i64,
    ) -> anyhow::Result<Vec<ProjectRecord>> {
        let scope = match scope {
            LibraryScope::Active => 0_i64,
            LibraryScope::Archived => 1_i64,
            LibraryScope::All => 2_i64,
        };
        let rows = sqlx::query(
            "SELECT id, name, description, instructions, created_at, updated_at, archived_at
             FROM projects WHERE owner_user_id = $1
               AND ($2 = 2 OR ($3 = 0 AND archived_at IS NULL)
                    OR ($4 = 1 AND archived_at IS NOT NULL))
             ORDER BY updated_at DESC LIMIT $5",
        )
        .bind(user_id)
        .bind(scope)
        .bind(scope)
        .bind(scope)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(project_from_row).collect())
    }

    pub async fn update_project(
        &self,
        user_id: &str,
        project_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        instructions: Option<&str>,
        archived: Option<bool>,
    ) -> anyhow::Result<ProjectRecord> {
        let existing = self
            .project(user_id, project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("项目不存在"))?;
        let next_name = name.unwrap_or(&existing.name);
        let next_description = description.unwrap_or(&existing.description);
        let next_instructions = instructions.unwrap_or(&existing.instructions);
        validate_project_fields(next_name, next_description, next_instructions)?;
        let now = unix_time();
        let archived_at = match archived {
            Some(true) => Some(now),
            Some(false) => None,
            None => existing.archived_at,
        };
        let status = if archived_at.is_some() {
            "archived"
        } else {
            "active"
        };
        sqlx::query(
            "UPDATE projects SET name = $1, description = $2, instructions = $3,
                status = $4, archived_at = $5, updated_at = $6
             WHERE id = $7 AND owner_user_id = $8",
        )
        .bind(next_name.trim())
        .bind(next_description.trim())
        .bind(next_instructions.trim())
        .bind(status)
        .bind(archived_at)
        .bind(now)
        .bind(project_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        self.project(user_id, project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("项目不存在"))
    }

    pub async fn create_project_conversation(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> anyhow::Result<String> {
        let active = sqlx::query(
            "SELECT 1 FROM projects
             WHERE id = $1 AND owner_user_id = $2 AND archived_at IS NULL",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .is_some();
        if !active {
            anyhow::bail!("项目不存在或已归档");
        }
        let id = format!("conv_{}", Uuid::new_v4().simple());
        let now = unix_time();
        sqlx::query(
            "INSERT INTO conversations(
                id, user_id, project_id, title, created_at, updated_at
             ) VALUES ($1, $2, $3, '新对话', $4, $5)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(project_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.touch_session(&id).await?;
        Ok(id)
    }

    pub async fn conversation_project_id(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<Option<Option<String>>> {
        let row =
            sqlx::query("SELECT project_id FROM conversations WHERE id = $1 AND user_id = $2")
                .bind(conversation_id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|row| row.get("project_id")))
    }

    pub async fn project_for_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> anyhow::Result<Option<ProjectRecord>> {
        let row = sqlx::query(
            "SELECT p.id, p.name, p.description, p.instructions,
                p.created_at, p.updated_at, p.archived_at
             FROM conversations c
             JOIN projects p ON p.id = c.project_id
             WHERE c.id = $1 AND c.user_id = $2 AND p.owner_user_id = $2",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(project_from_row))
    }

    pub async fn conversation_belongs_to(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM conversations WHERE id = $1 AND user_id = $2")
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
             FROM conversations c WHERE c.user_id = $1 AND c.project_id IS NULL
               AND ($2 = 2 OR ($3 = 0 AND c.archived_at IS NULL)
                    OR ($4 = 1 AND c.archived_at IS NOT NULL))
               AND ($5 = 0 OR c.is_pinned = 1)
               AND ($6 = '' OR c.title LIKE $7 OR
                    COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                              ORDER BY t.id DESC LIMIT 1), '') LIKE $8)
             ORDER BY c.is_pinned DESC, c.updated_at DESC LIMIT $9",
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

    pub async fn list_project_conversations(
        &self,
        user_id: &str,
        project_id: &str,
        scope: LibraryScope,
        limit: i64,
    ) -> anyhow::Result<Vec<ConversationSummary>> {
        if self.project(user_id, project_id).await?.is_none() {
            anyhow::bail!("项目不存在");
        }
        let scope = match scope {
            LibraryScope::Active => 0_i64,
            LibraryScope::Archived => 1_i64,
            LibraryScope::All => 2_i64,
        };
        let rows = sqlx::query(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                c.is_pinned, c.archived_at,
                COALESCE((SELECT content FROM turns t WHERE t.session_id = c.id
                          ORDER BY t.id DESC LIMIT 1), '') AS preview
             FROM conversations c
             WHERE c.user_id = $1 AND c.project_id = $2
               AND ($3 = 2 OR ($4 = 0 AND c.archived_at IS NULL)
                    OR ($5 = 1 AND c.archived_at IS NOT NULL))
             ORDER BY c.is_pinned DESC, c.updated_at DESC LIMIT $6",
        )
        .bind(user_id)
        .bind(project_id)
        .bind(scope)
        .bind(scope)
        .bind(scope)
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
             FROM conversations c WHERE c.user_id = $1 AND c.id = $2",
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
            "UPDATE conversations SET title = $1, updated_at = $2 WHERE id = $3 AND user_id = $4",
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
            QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM conversations WHERE user_id = ");
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
            let mut clear_sources = QueryBuilder::<Postgres>::new(
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

            let mut clear_todo_sources = QueryBuilder::<Postgres>::new(
                "UPDATE todos SET conversation_id = NULL, source_turn_id = NULL WHERE user_id = ",
            );
            clear_todo_sources
                .push_bind(user_id)
                .push(" AND conversation_id IN (");
            let mut separated = clear_todo_sources.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            clear_todo_sources
                .build()
                .execute(&mut *transaction)
                .await?;

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
                let mut delete = QueryBuilder::<Postgres>::new(prefix);
                let mut separated = delete.separated(", ");
                for id in ids {
                    separated.push_bind(id);
                }
                separated.push_unseparated(if nested { "))" } else { ")" });
                delete.build().execute(&mut *transaction).await?;
            }
        } else {
            let mut update = QueryBuilder::<Postgres>::new("UPDATE conversations SET ");
            match action {
                LibraryAction::Pin => {
                    update.push("is_pinned = CASE WHEN archived_at IS NULL THEN 1 ELSE 0 END")
                }
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
                QueryBuilder::<Postgres>::new("DELETE FROM conversations WHERE user_id = ");
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
            "SELECT id, role, content, created_at FROM turns WHERE session_id = $1
             ORDER BY id ASC LIMIT $2",
        )
        .bind(conversation_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let turn_id: i64 = row.get("id");
            let role: String = row.get("role");
            let attachment_rows = sqlx::query(
                "SELECT a.id, ta.memory_id, ta.todo_id, ta.caption
                 FROM turn_attachments ta JOIN assets a ON a.id = ta.asset_id
                 WHERE ta.turn_id = $1 AND a.user_id = $2 ORDER BY ta.ordinal ASC",
            )
            .bind(turn_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            let action_rows = if role == "user" {
                sqlx::query(
                    "SELECT kind, target_id, label, due_at FROM (
                        SELECT 'memory' AS kind, id AS target_id, user_note AS label,
                               NULL AS due_at, 0 AS kind_order, created_at
                        FROM memory_items WHERE user_id = $1 AND source_turn_id = $2
                        UNION ALL
                        SELECT 'todo' AS kind, id AS target_id, title AS label,
                               due_at, 1 AS kind_order, created_at
                        FROM todos WHERE user_id = $3 AND source_turn_id = $4
                     ) ORDER BY kind_order ASC, created_at ASC, target_id ASC LIMIT 10",
                )
                .bind(user_id)
                .bind(turn_id)
                .bind(user_id)
                .bind(turn_id)
                .fetch_all(&self.pool)
                .await?
            } else {
                Vec::new()
            };
            messages.push(ConversationMessage {
                id: turn_id,
                role,
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
                            todo_id: attachment.get("todo_id"),
                            caption: attachment.get("caption"),
                        }
                    })
                    .collect(),
                actions: action_rows
                    .into_iter()
                    .map(|action| ConversationAction {
                        kind: action.get("kind"),
                        target_id: action.get("target_id"),
                        label: action.get("label"),
                        due_at: action.get("due_at"),
                    })
                    .collect(),
            });
        }
        Ok(Some(messages))
    }

    pub async fn touch_session(&self, session_id: &str) -> anyhow::Result<()> {
        let now = unix_time();
        sqlx::query(
            "INSERT INTO sessions(id, created_at, updated_at) VALUES ($1, $2, $3)
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
            "INSERT INTO events(session_id, kind, payload, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id)
        .bind(kind)
        .bind(serde_json::to_string(payload)?)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE sessions SET updated_at = $1 WHERE id = $2")
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
        let row = sqlx::query(
            "INSERT INTO turns(session_id, role, content, metadata, created_at)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id",
        )
        .bind(session_id)
        .bind(role)
        .bind(content)
        .bind(serde_json::to_string(
            metadata.unwrap_or(&Value::Object(Default::default())),
        )?)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE conversations SET updated_at = $1,
             title = CASE WHEN title = '新对话' AND $2 = 'user'
                          THEN substr($3, 1, 32) ELSE title END
             WHERE id = $4",
        )
        .bind(now)
        .bind(role)
        .bind(content.trim())
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn readiness(&self) -> anyhow::Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn recent_messages(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT role, content FROM turns WHERE session_id = $1
             ORDER BY id DESC LIMIT $2",
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
            "SELECT role, content FROM turns WHERE session_id = $1 ORDER BY id DESC LIMIT $2",
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
        let row = sqlx::query(
            "INSERT INTO memories(session_id, content, created_at)
                 VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(session_id)
        .bind(content)
        .bind(unix_time())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn recent_memories(
        &self,
        session_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT content FROM memories WHERE session_id = $1 ORDER BY id DESC LIMIT $2",
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

    pub async fn relevant_explicit_memories(
        &self,
        user_id: &str,
        conversation_id: &str,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<String>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "WITH current_scope AS (
                SELECT project_id FROM conversations
                WHERE id = $1 AND user_id = $2
             )
             SELECT m.user_note, m.visual_summary, m.scope_type,
                    similarity(m.user_note || ' ' || m.visual_summary, $3) AS score
             FROM memory_items m
             CROSS JOIN current_scope s
             WHERE m.user_id = $2 AND m.archived_at IS NULL
               AND (
                    (s.project_id IS NULL AND m.scope_type = 'personal')
                    OR (s.project_id IS NOT NULL AND
                        (m.scope_type = 'personal' OR m.project_id = s.project_id))
               )
               AND (
                    similarity(m.user_note || ' ' || m.visual_summary, $3) >= 0.08
                    OR m.user_note ILIKE '%' || $3 || '%'
                    OR m.visual_summary ILIKE '%' || $3 || '%'
               )
             ORDER BY m.is_pinned DESC, score DESC, m.created_at DESC
             LIMIT $4",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(query)
        .bind(limit.clamp(1, 20))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let note: String = row.get("user_note");
                let visual: String = row.get("visual_summary");
                let scope: String = row.get("scope_type");
                let label = if scope == "project" {
                    "项目显式记忆"
                } else {
                    "个人显式记忆"
                };
                if visual.trim().is_empty() {
                    format!("[{label}] {note}")
                } else {
                    format!("[{label}] {note}；画面：{visual}")
                }
            })
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
                "SELECT content FROM memories WHERE session_id = $1
                 ORDER BY id DESC LIMIT $2",
            )
            .bind(session_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT content FROM memories WHERE session_id = $1 AND content LIKE $2
                 ORDER BY id DESC LIMIT $3",
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

fn project_from_row(row: sqlx::postgres::PgRow) -> ProjectRecord {
    ProjectRecord {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        instructions: row.get("instructions"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        archived_at: row.get("archived_at"),
    }
}

fn user_profile_from_row(row: sqlx::postgres::PgRow) -> UserProfile {
    UserProfile {
        ai_identity: row.get("ai_identity"),
        user_identity: row.get("user_identity"),
        preferred_name: row.get("preferred_name"),
        basic_memory: row.get("basic_memory"),
        updated_at: Some(row.get("updated_at")),
    }
}

fn validate_user_profile(
    ai_identity: &str,
    user_identity: &str,
    preferred_name: &str,
    basic_memory: &str,
) -> anyhow::Result<()> {
    if ai_identity.trim().chars().count() > 2_000 {
        anyhow::bail!("Ripple 身份设定不能超过 2000 个字符");
    }
    if user_identity.trim().chars().count() > 2_000 {
        anyhow::bail!("使用者身份设定不能超过 2000 个字符");
    }
    if preferred_name.trim().chars().count() > 80 {
        anyhow::bail!("称呼不能超过 80 个字符");
    }
    if basic_memory.trim().chars().count() > 4_000 {
        anyhow::bail!("基础资料不能超过 4000 个字符");
    }
    Ok(())
}

fn validate_project_fields(
    name: &str,
    description: &str,
    instructions: &str,
) -> anyhow::Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        anyhow::bail!("项目名称不能为空且不能超过 80 个字符");
    }
    if description.trim().chars().count() > 2_000 {
        anyhow::bail!("项目说明不能超过 2000 个字符");
    }
    if instructions.trim().chars().count() > 4_000 {
        anyhow::bail!("项目规则不能超过 4000 个字符");
    }
    Ok(())
}

async fn insert_auth_session(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    now: f64,
    token_ttl_hours: i64,
) -> anyhow::Result<String> {
    let token = new_access_token();
    let expires_at = now + token_ttl_hours.clamp(1, 24 * 365) as f64 * 3600.0;
    sqlx::query(
        "INSERT INTO auth_sessions(token_hash, user_id, created_at, expires_at)
         VALUES ($1, $2, $3, $4)",
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
    async fn stores_user_profile_per_user() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("profile.sqlite3"))
            .await
            .unwrap();
        let user = registered_user(&store, "profile@example.com", "profile-invite").await;

        assert_eq!(
            store.user_profile(&user.id).await.unwrap(),
            UserProfile::default()
        );
        let profile = store
            .update_user_profile(
                &user.id,
                "你是 Ripple，一位温柔直接的长期伙伴",
                "我是独立开发者",
                "Lake",
                "我偏好先看结论，再看细节",
            )
            .await
            .unwrap();

        assert_eq!(profile.preferred_name, "Lake");
        assert!(profile.updated_at.is_some());
        assert_eq!(store.user_profile(&user.id).await.unwrap(), profile);
    }

    #[tokio::test]
    async fn conversation_messages_include_source_actions() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("actions.sqlite3"))
            .await
            .unwrap();
        let user = registered_user(&store, "actions@example.com", "actions-invite").await;
        let other =
            registered_user(&store, "other-actions@example.com", "other-actions-invite").await;
        let conversation = store.create_conversation(&user.id).await.unwrap();
        let user_turn = store
            .add_turn(&conversation, "user", "记住并提醒我", None)
            .await
            .unwrap();
        store
            .add_turn(&conversation, "assistant", "已经完成", None)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO memory_items(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                user_note, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind("mem-action")
        .bind(&user.id)
        .bind(&conversation)
        .bind(user_turn)
        .bind("response-action")
        .bind("蓝色转接头在书桌抽屉")
        .bind(2.0)
        .bind(2.0)
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_items(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                user_note, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind("foreign-action")
        .bind(&other.id)
        .bind(&conversation)
        .bind(user_turn)
        .bind("response-foreign")
        .bind("不应泄漏")
        .bind(1.0)
        .bind(1.0)
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO todos(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                title, due_at, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind("todo-action")
        .bind(&user.id)
        .bind(&conversation)
        .bind(user_turn)
        .bind("response-action")
        .bind("周一带充电器")
        .bind(1_900_000_000.0)
        .bind(1.0)
        .bind(1.0)
        .execute(store.pool())
        .await
        .unwrap();

        let messages = store
            .conversation_messages(&user.id, &conversation, 20)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages[0].actions.len(), 2);
        assert_eq!(messages[0].actions[0].kind, "memory");
        assert_eq!(messages[0].actions[1].kind, "todo");
        assert_eq!(messages[0].actions[1].label, "周一带充电器");
        assert_eq!(messages[0].actions[1].due_at, Some(1_900_000_000.0));
        assert!(messages[1].actions.is_empty());

        for index in 0..9 {
            sqlx::query(
                "INSERT INTO memory_items(
                    id, user_id, conversation_id, source_turn_id, source_response_id,
                    user_note, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(format!("mem-cap-{index}"))
            .bind(&user.id)
            .bind(&conversation)
            .bind(user_turn)
            .bind(format!("response-cap-{index}"))
            .bind(format!("记忆 {index}"))
            .bind(10.0 + f64::from(index))
            .bind(10.0 + f64::from(index))
            .execute(store.pool())
            .await
            .unwrap();
        }
        let capped = store
            .conversation_messages(&user.id, &conversation, 20)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(capped[0].actions.len(), 10);
        assert!(
            capped[0]
                .actions
                .iter()
                .all(|action| action.kind == "memory")
        );
        assert_eq!(capped[0].actions[0].target_id, "mem-action");
        assert_eq!(capped[0].actions[9].target_id, "mem-cap-8");
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
    async fn projects_isolate_their_conversations_from_personal_chat() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("projects.pg-test"))
            .await
            .unwrap();
        let user = registered_user(&store, "projects@example.com", "projects-invite").await;
        let personal = store.create_conversation(&user.id).await.unwrap();
        let project = store
            .create_project(
                &user.id,
                "Ripple Live",
                "Android 优先的多模态 Agent",
                "所有模型调用使用 Responses API",
            )
            .await
            .unwrap();
        let project_conversation = store
            .create_project_conversation(&user.id, &project.id)
            .await
            .unwrap();

        let personal_rows = store
            .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
            .await
            .unwrap();
        assert_eq!(personal_rows.len(), 1);
        assert_eq!(personal_rows[0].id, personal);

        let project_rows = store
            .list_project_conversations(&user.id, &project.id, LibraryScope::Active, 50)
            .await
            .unwrap();
        assert_eq!(project_rows.len(), 1);
        assert_eq!(project_rows[0].id, project_conversation);
        assert_eq!(
            store
                .conversation_project_id(&user.id, &project_conversation)
                .await
                .unwrap(),
            Some(Some(project.id.clone()))
        );

        let loaded = store
            .project_for_conversation(&user.id, &project_conversation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.name, "Ripple Live");
        assert!(loaded.instructions.contains("Responses API"));
    }

    #[tokio::test]
    async fn deleting_conversation_detaches_todo_sources_before_removing_turns() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContextStore::open(&directory.path().join("delete.sqlite3"))
            .await
            .unwrap();
        let user = registered_user(&store, "delete@example.com", "delete-invite").await;
        let conversation = store.create_conversation(&user.id).await.unwrap();
        let source_turn = store
            .add_turn(&conversation, "user", "周一带充电器", None)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO todos(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                title, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind("todo-delete")
        .bind(&user.id)
        .bind(&conversation)
        .bind(source_turn)
        .bind("response-delete")
        .bind("周一带充电器")
        .bind(1.0)
        .bind(1.0)
        .execute(store.pool())
        .await
        .unwrap();

        store
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation),
                LibraryAction::Delete,
            )
            .await
            .unwrap();

        assert!(
            store
                .conversation_messages(&user.id, &conversation, 20)
                .await
                .unwrap()
                .is_none()
        );
        let todo_sources: (Option<String>, Option<i64>) =
            sqlx::query_as("SELECT conversation_id, source_turn_id FROM todos WHERE id = $1")
                .bind("todo-delete")
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(todo_sources, (None, None));
    }
}
