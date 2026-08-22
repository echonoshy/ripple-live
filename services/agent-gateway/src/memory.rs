use std::{cmp::Ordering, collections::HashSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;

use crate::{
    asset_store::AssetStore,
    auth::avatar_content_url,
    context::{ContextStore, LibraryAction, LibraryScope},
    protocol::VideoFrame,
};

#[derive(Clone)]
pub struct MemoryService {
    context: ContextStore,
    assets: AssetStore,
}

#[derive(Clone, Debug)]
pub struct CreateMemoryRequest {
    pub user_id: String,
    pub conversation_id: String,
    pub source_turn_id: i64,
    pub response_id: String,
    pub tool_call_id: String,
    pub user_note: String,
    pub visual_summary: String,
    pub frames: Vec<VideoFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryArtifact {
    pub id: String,
    pub kind: String,
    pub memory_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub caption: String,
    pub content_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryRecord {
    pub id: String,
    pub kind: String,
    pub user_note: String,
    pub visual_summary: String,
    pub captured_at: Option<f64>,
    pub created_at: f64,
    pub cover: Option<MemoryArtifact>,
    pub is_pinned: bool,
    pub archived_at: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemorySearchResult {
    #[serde(flatten)]
    pub memory: MemoryRecord,
    pub score: f64,
    pub assets: Vec<MemoryArtifact>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryFactRecord {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub scope_type: String,
    pub project_id: Option<String>,
    pub source: String,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Clone, Debug)]
pub struct CreateTodoRequest {
    pub user_id: String,
    pub conversation_id: String,
    pub source_turn_id: i64,
    pub response_id: String,
    pub tool_call_id: String,
    pub title: String,
    pub visual_summary: String,
    pub due_at: Option<f64>,
    pub frames: Vec<VideoFrame>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TodoRecord {
    pub id: String,
    pub memory_id: Option<String>,
    pub title: String,
    pub visual_summary: String,
    pub due_at: Option<f64>,
    pub completed_at: Option<f64>,
    pub created_at: f64,
    pub cover: Option<MemoryArtifact>,
}

#[derive(Clone, Debug, Default)]
pub struct TodoUpdate {
    pub title: Option<String>,
    pub due_at: Option<Option<f64>>,
    pub completed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemoryRequest {
    pub user_note: String,
}

pub struct AssetContent {
    pub path: PathBuf,
    pub mime_type: String,
}

impl MemoryService {
    pub async fn new(context: ContextStore, assets_dir: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            context,
            assets: AssetStore::new(assets_dir).await?,
        })
    }

    pub async fn list_facts(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryFactRecord>> {
        let query = query.trim();
        let rows = sqlx::query(
            "SELECT f.id, f.kind, f.summary, f.scope_type, f.project_id,
                    f.created_at, f.updated_at,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM memory_evidence e
                        WHERE e.fact_id = f.id AND e.turn_id IS NOT NULL
                    ) THEN 'conversation' ELSE 'manual' END AS source
             FROM memory_facts f
             WHERE f.user_id = $1 AND f.activation_status = 'active'
               AND ($2 = '' OR f.summary ILIKE '%' || $2 || '%')
             ORDER BY f.salience DESC, f.updated_at DESC
             LIMIT $3",
        )
        .bind(user_id)
        .bind(query)
        .bind(limit.clamp(1, 100) as i64)
        .fetch_all(self.context.pool())
        .await?;
        Ok(rows.into_iter().map(memory_fact_from_row).collect())
    }

    pub async fn create_manual_fact(
        &self,
        user_id: &str,
        kind: &str,
        summary: &str,
    ) -> anyhow::Result<MemoryFactRecord> {
        self.upsert_explicit_fact(user_id, None, kind, summary, None, None)
            .await
    }

    pub async fn remember_fact_from_memory(
        &self,
        user_id: &str,
        conversation_id: &str,
        source_turn_id: i64,
        memory_item_id: &str,
        kind: &str,
        summary: &str,
    ) -> anyhow::Result<MemoryFactRecord> {
        let project_id = self
            .context
            .conversation_project_id(user_id, conversation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("对话不存在"))?;
        self.upsert_explicit_fact(
            user_id,
            project_id.as_deref(),
            kind,
            summary,
            Some(source_turn_id),
            Some(memory_item_id),
        )
        .await
    }

    pub async fn update_fact(
        &self,
        user_id: &str,
        fact_id: &str,
        kind: Option<&str>,
        summary: Option<&str>,
    ) -> anyhow::Result<Option<MemoryFactRecord>> {
        let Some(current) = self.get_fact(user_id, fact_id).await? else {
            return Ok(None);
        };
        let next_kind = kind.unwrap_or(&current.kind);
        let next_summary = summary.unwrap_or(&current.summary);
        validate_fact(next_kind, next_summary)?;
        sqlx::query(
            "UPDATE memory_facts
             SET kind = $1, summary = $2, canonical_key = $3, updated_at = $4
             WHERE id = $5 AND user_id = $6 AND activation_status = 'active'",
        )
        .bind(next_kind)
        .bind(next_summary.trim())
        .bind(canonical_fact_key(next_summary))
        .bind(unix_time())
        .bind(fact_id)
        .bind(user_id)
        .execute(self.context.pool())
        .await?;
        self.get_fact(user_id, fact_id).await
    }

    pub async fn delete_fact(&self, user_id: &str, fact_id: &str) -> anyhow::Result<bool> {
        Ok(
            sqlx::query("DELETE FROM memory_facts WHERE id = $1 AND user_id = $2")
                .bind(fact_id)
                .bind(user_id)
                .execute(self.context.pool())
                .await?
                .rows_affected()
                > 0,
        )
    }

    async fn upsert_explicit_fact(
        &self,
        user_id: &str,
        project_id: Option<&str>,
        kind: &str,
        summary: &str,
        source_turn_id: Option<i64>,
        memory_item_id: Option<&str>,
    ) -> anyhow::Result<MemoryFactRecord> {
        validate_fact(kind, summary)?;
        let summary = summary.trim();
        let canonical_key = canonical_fact_key(summary);
        let scope_type = if project_id.is_some() {
            "project"
        } else {
            "personal"
        };
        let now = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT id FROM memory_facts
             WHERE user_id = $1 AND scope_type = $2
               AND project_id IS NOT DISTINCT FROM $3
               AND canonical_key = $4 AND activation_status = 'active'
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(scope_type)
        .bind(project_id)
        .bind(&canonical_key)
        .fetch_optional(&mut *transaction)
        .await?;
        let fact_id = existing.unwrap_or_else(|| format!("fact_{}", Uuid::new_v4().simple()));
        sqlx::query(
            "INSERT INTO memory_facts(
                id, user_id, scope_type, project_id, kind, canonical_key,
                summary, salience, confidence, explicit, activation_status,
                created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, 1.0, 1.0, 1, 'active', $8, $8)
             ON CONFLICT(id) DO UPDATE SET
                kind = EXCLUDED.kind, summary = EXCLUDED.summary,
                salience = 1.0, confidence = 1.0, explicit = 1,
                activation_status = 'active', updated_at = EXCLUDED.updated_at",
        )
        .bind(&fact_id)
        .bind(user_id)
        .bind(scope_type)
        .bind(project_id)
        .bind(kind)
        .bind(&canonical_key)
        .bind(summary)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        if source_turn_id.is_some() || memory_item_id.is_some() {
            sqlx::query(
                "INSERT INTO memory_evidence(fact_id, turn_id, memory_item_id)
                 SELECT $1, $2, $3
                 WHERE NOT EXISTS (
                    SELECT 1 FROM memory_evidence
                    WHERE fact_id = $1
                      AND turn_id IS NOT DISTINCT FROM $2
                      AND memory_item_id IS NOT DISTINCT FROM $3
                 )",
            )
            .bind(&fact_id)
            .bind(source_turn_id)
            .bind(memory_item_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.get_fact(user_id, &fact_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的记忆不存在"))
    }

    async fn get_fact(
        &self,
        user_id: &str,
        fact_id: &str,
    ) -> anyhow::Result<Option<MemoryFactRecord>> {
        let row = sqlx::query(
            "SELECT f.id, f.kind, f.summary, f.scope_type, f.project_id,
                    f.created_at, f.updated_at,
                    CASE WHEN EXISTS (
                        SELECT 1 FROM memory_evidence e
                        WHERE e.fact_id = f.id AND e.turn_id IS NOT NULL
                    ) THEN 'conversation' ELSE 'manual' END AS source
             FROM memory_facts f
             WHERE f.id = $1 AND f.user_id = $2 AND f.activation_status = 'active'",
        )
        .bind(fact_id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        Ok(row.map(memory_fact_from_row))
    }

    pub async fn set_avatar(&self, user_id: &str, bytes: &[u8]) -> anyhow::Result<String> {
        let stored = self.assets.store_jpeg(user_id, bytes).await?;
        let now = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        let previous = sqlx::query("SELECT avatar_asset_id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| anyhow::anyhow!("用户不存在"))?
            .get::<Option<String>, _>("avatar_asset_id");
        sqlx::query(
            "INSERT INTO assets(
                id, user_id, sha256, mime_type, storage_key, width, height,
                size_bytes, captured_at, created_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9)
             ON CONFLICT(user_id, sha256) DO NOTHING",
        )
        .bind(&stored.id)
        .bind(user_id)
        .bind(&stored.sha256)
        .bind(&stored.mime_type)
        .bind(&stored.storage_key)
        .bind(i64::from(stored.width))
        .bind(i64::from(stored.height))
        .bind(stored.size_bytes as i64)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = sqlx::query("SELECT id FROM assets WHERE user_id = $1 AND sha256 = $2")
            .bind(user_id)
            .bind(&stored.sha256)
            .fetch_one(&mut *transaction)
            .await?;
        let asset_id = row.get::<String, _>("id");
        sqlx::query("UPDATE users SET avatar_asset_id = $1 WHERE id = $2")
            .bind(&asset_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        if let Some(previous) = previous.filter(|id| id != &asset_id) {
            self.remove_asset_if_unreferenced(user_id, &previous)
                .await?;
        }
        Ok(avatar_content_url(&asset_id))
    }

    pub async fn clear_avatar(&self, user_id: &str) -> anyhow::Result<bool> {
        let mut transaction = self.context.pool().begin().await?;
        let previous = sqlx::query("SELECT avatar_asset_id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| anyhow::anyhow!("用户不存在"))?
            .get::<Option<String>, _>("avatar_asset_id");
        let Some(previous) = previous else {
            transaction.commit().await?;
            return Ok(false);
        };
        sqlx::query("UPDATE users SET avatar_asset_id = NULL WHERE id = $1")
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.remove_asset_if_unreferenced(user_id, &previous)
            .await?;
        Ok(true)
    }

    async fn remove_asset_if_unreferenced(
        &self,
        user_id: &str,
        asset_id: &str,
    ) -> anyhow::Result<()> {
        let references: i64 = sqlx::query_scalar(
            "SELECT
                (SELECT COUNT(*) FROM memory_assets WHERE asset_id = $1) +
                (SELECT COUNT(*) FROM memory_items WHERE cover_asset_id = $1) +
                (SELECT COUNT(*) FROM turn_attachments WHERE asset_id = $1) +
                (SELECT COUNT(*) FROM todos WHERE cover_asset_id = $1) +
                (SELECT COUNT(*) FROM project_resources WHERE asset_id = $1) +
                (SELECT COUNT(*) FROM library_resources WHERE asset_id = $1) +
                (SELECT COUNT(*) FROM memory_evidence WHERE asset_id = $1) +
                (SELECT COUNT(*) FROM users WHERE avatar_asset_id = $1)",
        )
        .bind(asset_id)
        .fetch_one(self.context.pool())
        .await?;
        if references != 0 {
            return Ok(());
        }
        let storage_key = sqlx::query_scalar::<_, String>(
            "DELETE FROM assets WHERE id = $1 AND user_id = $2 RETURNING storage_key",
        )
        .bind(asset_id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        if let Some(storage_key) = storage_key {
            self.assets.remove(&storage_key).await?;
        }
        Ok(())
    }

    pub async fn create(&self, request: CreateMemoryRequest) -> anyhow::Result<MemorySearchResult> {
        if let Some(memory_id) = self
            .existing_execution(&request.response_id, &request.tool_call_id)
            .await?
        {
            return self
                .get(&request.user_id, &memory_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("幂等记录指向的记忆不存在"));
        }
        let note = request.user_note.trim();
        if note.is_empty() || note.chars().count() > 2_000 {
            anyhow::bail!("记忆内容不能为空且不能超过 2000 个字符");
        }
        let visual_summary = request.visual_summary.trim();
        if visual_summary.chars().count() > 2_000 {
            anyhow::bail!("画面描述不能超过 2000 个字符");
        }
        let project_id = self
            .context
            .conversation_project_id(&request.user_id, &request.conversation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("对话不存在"))?;
        let scope_type = if project_id.is_some() {
            "project"
        } else {
            "personal"
        };

        let mut stored = Vec::new();
        for frame in request.frames.iter().take(3) {
            if frame.mime_type != "image/jpeg" {
                continue;
            }
            let asset = self
                .assets
                .store_jpeg(&request.user_id, &frame.bytes)
                .await?;
            stored.push((asset, frame.captured_at_ms));
        }

        let memory_id = format!("mem_{}", Uuid::new_v4().simple());
        let now = unix_time();
        let captured_at = stored
            .iter()
            .filter_map(|(_, captured)| *captured)
            .max()
            .map(|value| value as f64 / 1_000.0);
        let mut transaction = self.context.pool().begin().await?;
        let mut asset_ids = Vec::new();
        for (asset, frame_captured_at) in &stored {
            sqlx::query(
                "INSERT INTO assets(
                    id, user_id, sha256, mime_type, storage_key, width, height,
                    size_bytes, captured_at, created_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(user_id, sha256) DO NOTHING",
            )
            .bind(&asset.id)
            .bind(&request.user_id)
            .bind(&asset.sha256)
            .bind(&asset.mime_type)
            .bind(&asset.storage_key)
            .bind(i64::from(asset.width))
            .bind(i64::from(asset.height))
            .bind(asset.size_bytes as i64)
            .bind(frame_captured_at.map(|value| value as f64 / 1_000.0))
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            let row = sqlx::query("SELECT id FROM assets WHERE user_id = $1 AND sha256 = $2")
                .bind(&request.user_id)
                .bind(&asset.sha256)
                .fetch_one(&mut *transaction)
                .await?;
            asset_ids.push(row.get::<String, _>("id"));
        }
        let cover_asset_id = asset_ids.last().cloned();
        sqlx::query(
            "INSERT INTO memory_items(
                id, user_id, scope_type, project_id,
                conversation_id, source_turn_id, source_response_id,
                kind, user_note, visual_summary, cover_asset_id, captured_at,
                created_at, updated_at
             ) VALUES (
                $1, $2, $3, $4, $5, $6, $7,
                $8, $9, $10, $11, $12, $13, $14
             )",
        )
        .bind(&memory_id)
        .bind(&request.user_id)
        .bind(scope_type)
        .bind(&project_id)
        .bind(&request.conversation_id)
        .bind(request.source_turn_id)
        .bind(&request.response_id)
        .bind(if asset_ids.is_empty() {
            "text"
        } else {
            "visual"
        })
        .bind(note)
        .bind(visual_summary)
        .bind(&cover_asset_id)
        .bind(captured_at)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        for (ordinal, asset_id) in asset_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO memory_assets(memory_id, asset_id, ordinal, is_cover)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&memory_id)
            .bind(asset_id)
            .bind(ordinal as i64)
            .bind(i64::from(Some(asset_id) == cover_asset_id.as_ref()))
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO memory_tool_executions(response_id, tool_call_id, memory_id)
             VALUES ($1, $2, $3)",
        )
        .bind(&request.response_id)
        .bind(&request.tool_call_id)
        .bind(&memory_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        self.get(&request.user_id, &memory_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的记忆不存在"))
    }

    pub async fn recall(
        &self,
        user_id: &str,
        conversation_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySearchResult>> {
        let project_id = match conversation_id {
            Some(conversation_id) => self
                .context
                .conversation_project_id(user_id, conversation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("对话不存在"))?,
            None => None,
        };
        let rows = sqlx::query(
            "SELECT id FROM memory_items
             WHERE user_id = $1 AND archived_at IS NULL
               AND (
                    ($2::text IS NULL AND scope_type = 'personal')
                    OR ($2::text IS NOT NULL AND
                        (scope_type = 'personal' OR project_id = $2))
               )
             ORDER BY created_at DESC LIMIT 200",
        )
        .bind(user_id)
        .bind(project_id.as_deref())
        .fetch_all(self.context.pool())
        .await?;
        let mut results = Vec::new();
        for row in rows {
            let id: String = row.get("id");
            if let Some(mut memory) = self.get(user_id, &id).await? {
                memory.score = relevance(
                    query,
                    &format!(
                        "{} {}",
                        memory.memory.user_note, memory.memory.visual_summary
                    ),
                );
                results.push(memory);
            }
        }
        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.memory.created_at.total_cmp(&left.memory.created_at))
        });
        if !query.trim().is_empty() {
            results.retain(|item| item.score > 0.0);
        }
        results.truncate(limit.clamp(1, 20));
        Ok(results)
    }

    /// Todo evidence is stored on the todo itself. It must not create an entry
    /// in the user's long-term visual-memory library.
    pub async fn create_todo(&self, request: CreateTodoRequest) -> anyhow::Result<TodoRecord> {
        if let Some(todo_id) = self
            .existing_todo_execution(&request.response_id, &request.tool_call_id)
            .await?
        {
            return self
                .get_todo(&request.user_id, &todo_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("幂等记录指向的待办不存在"));
        }
        let title = request.title.trim();
        if title.is_empty() || title.chars().count() > 500 {
            anyhow::bail!("待办标题不能为空且不能超过 500 个字符");
        }
        let summary = request.visual_summary.trim();
        if summary.chars().count() > 2_000 {
            anyhow::bail!("待办摘要不能超过 2000 个字符");
        }
        let stored = match request
            .frames
            .iter()
            .rev()
            .find(|frame| frame.mime_type == "image/jpeg")
        {
            Some(frame) => Some((
                self.assets
                    .store_jpeg(&request.user_id, &frame.bytes)
                    .await?,
                frame.captured_at_ms,
            )),
            None => None,
        };
        let todo_id = format!("todo_{}", Uuid::new_v4().simple());
        let now = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        let cover_asset_id = if let Some((asset, captured_at_ms)) = stored {
            sqlx::query(
                "INSERT INTO assets(
                    id, user_id, sha256, mime_type, storage_key, width, height,
                    size_bytes, captured_at, created_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(user_id, sha256) DO NOTHING",
            )
            .bind(&asset.id)
            .bind(&request.user_id)
            .bind(&asset.sha256)
            .bind(&asset.mime_type)
            .bind(&asset.storage_key)
            .bind(i64::from(asset.width))
            .bind(i64::from(asset.height))
            .bind(asset.size_bytes as i64)
            .bind(captured_at_ms.map(|value| value as f64 / 1_000.0))
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            Some(
                sqlx::query_scalar::<_, String>(
                    "SELECT id FROM assets WHERE user_id = $1 AND sha256 = $2",
                )
                .bind(&request.user_id)
                .bind(&asset.sha256)
                .fetch_one(&mut *transaction)
                .await?,
            )
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO todos(
                id, user_id, memory_id, conversation_id, source_turn_id,
                source_response_id, title, visual_summary, cover_asset_id,
                due_at, completed_at, created_at, updated_at
             ) VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, $9, NULL, $10, $11)",
        )
        .bind(&todo_id)
        .bind(&request.user_id)
        .bind(&request.conversation_id)
        .bind(request.source_turn_id)
        .bind(&request.response_id)
        .bind(title)
        .bind(summary)
        .bind(&cover_asset_id)
        .bind(request.due_at)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO todo_tool_executions(response_id, tool_call_id, todo_id)
             VALUES ($1, $2, $3)",
        )
        .bind(&request.response_id)
        .bind(&request.tool_call_id)
        .bind(&todo_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_todo(&request.user_id, &todo_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的待办不存在"))
    }

    pub async fn list_todos(
        &self,
        user_id: &str,
        completed: Option<bool>,
        limit: usize,
    ) -> anyhow::Result<Vec<TodoRecord>> {
        let rows = match completed {
            Some(true) => sqlx::query(
                "SELECT id FROM todos WHERE user_id = $1 AND completed_at IS NOT NULL
                 ORDER BY completed_at DESC LIMIT $2",
            )
            .bind(user_id)
            .bind(limit.clamp(1, 100) as i64)
            .fetch_all(self.context.pool())
            .await?,
            _ => sqlx::query(
                "SELECT id FROM todos WHERE user_id = $1 AND completed_at IS NULL
                 ORDER BY CASE WHEN due_at IS NULL THEN 1 ELSE 0 END, due_at ASC, created_at DESC LIMIT $2",
            )
            .bind(user_id)
            .bind(limit.clamp(1, 100) as i64)
            .fetch_all(self.context.pool())
            .await?,
        };
        let mut todos = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(todo) = self.get_todo(user_id, &row.get::<String, _>("id")).await? {
                todos.push(todo);
            }
        }
        Ok(todos)
    }

    pub async fn create_manual_todo(
        &self,
        user_id: &str,
        title: &str,
        due_at: Option<f64>,
    ) -> anyhow::Result<TodoRecord> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 500 {
            anyhow::bail!("待办标题不能为空且不能超过 500 个字符");
        }
        let todo_id = format!("todo_{}", Uuid::new_v4().simple());
        let now = unix_time();
        sqlx::query(
            "INSERT INTO todos(
                id, user_id, title, visual_summary, due_at, completed_at, created_at, updated_at
             ) VALUES ($1, $2, $3, '', $4, NULL, $5, $6)",
        )
        .bind(&todo_id)
        .bind(user_id)
        .bind(title)
        .bind(due_at)
        .bind(now)
        .bind(now)
        .execute(self.context.pool())
        .await?;
        self.get_todo(user_id, &todo_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的待办不存在"))
    }

    pub async fn update_todo(
        &self,
        user_id: &str,
        todo_id: &str,
        update: TodoUpdate,
    ) -> anyhow::Result<Option<TodoRecord>> {
        if update.title.is_none() && update.due_at.is_none() && update.completed.is_none() {
            anyhow::bail!("至少提供一项待办修改内容");
        }
        let title = update.title.as_deref().map(str::trim);
        if let Some(title) = title {
            if title.is_empty() || title.chars().count() > 500 {
                anyhow::bail!("待办标题不能为空且不能超过 500 个字符");
            }
        }
        let now = unix_time();
        let title_changed = title.is_some();
        let due_changed = update.due_at.is_some();
        let due_at = update.due_at.flatten();
        let completed_changed = update.completed.is_some();
        let completed_at = update.completed.filter(|completed| *completed).map(|_| now);
        let result = sqlx::query(
            "UPDATE todos SET
                title = CASE WHEN $1 THEN $2 ELSE title END,
                due_at = CASE WHEN $3 THEN $4 ELSE due_at END,
                completed_at = CASE WHEN $5 THEN $6 ELSE completed_at END,
                updated_at = $7
             WHERE id = $8 AND user_id = $9",
        )
        .bind(title_changed)
        .bind(title)
        .bind(due_changed)
        .bind(due_at)
        .bind(completed_changed)
        .bind(completed_at)
        .bind(now)
        .bind(todo_id)
        .bind(user_id)
        .execute(self.context.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_todo(user_id, todo_id).await
    }

    pub async fn complete_todo(
        &self,
        user_id: &str,
        todo_id: &str,
        completed: bool,
    ) -> anyhow::Result<Option<TodoRecord>> {
        self.update_todo(
            user_id,
            todo_id,
            TodoUpdate {
                completed: Some(completed),
                ..TodoUpdate::default()
            },
        )
        .await
    }

    pub async fn delete_todo(&self, user_id: &str, todo_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM todos WHERE id = $1 AND user_id = $2")
            .bind(todo_id)
            .bind(user_id)
            .execute(self.context.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list(
        &self,
        user_id: &str,
        scope: LibraryScope,
        pinned_only: bool,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        let scope = match scope {
            LibraryScope::Active => 0_i64,
            LibraryScope::Archived => 1_i64,
            LibraryScope::All => 2_i64,
        };
        let query = query.trim();
        let pattern = format!("%{query}%");
        let rows = sqlx::query(
            "SELECT id FROM memory_items WHERE user_id = $1
               AND scope_type = 'personal'
               AND ($2 = 2 OR ($3 = 0 AND archived_at IS NULL)
                    OR ($4 = 1 AND archived_at IS NOT NULL))
               AND ($5 = 0 OR is_pinned = 1)
               AND ($6 = '' OR user_note LIKE $7 OR visual_summary LIKE $8)
             ORDER BY is_pinned DESC, COALESCE(captured_at, created_at) DESC LIMIT $9",
        )
        .bind(user_id)
        .bind(scope)
        .bind(scope)
        .bind(scope)
        .bind(i64::from(pinned_only))
        .bind(query)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit.clamp(1, 100) as i64)
        .fetch_all(self.context.pool())
        .await?;
        let mut memories = Vec::new();
        for row in rows {
            if let Some(result) = self.get(user_id, &row.get::<String, _>("id")).await? {
                memories.push(result.memory);
            }
        }
        Ok(memories)
    }

    pub async fn get(
        &self,
        user_id: &str,
        memory_id: &str,
    ) -> anyhow::Result<Option<MemorySearchResult>> {
        let row = sqlx::query(
            "SELECT id, kind, user_note, visual_summary, cover_asset_id,
                    captured_at, created_at, is_pinned, archived_at
             FROM memory_items WHERE id = $1 AND user_id = $2",
        )
        .bind(memory_id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let asset_rows = sqlx::query(
            "SELECT a.id FROM memory_assets ma JOIN assets a ON a.id = ma.asset_id
             WHERE ma.memory_id = $1 AND a.user_id = $2 ORDER BY ma.ordinal ASC",
        )
        .bind(memory_id)
        .bind(user_id)
        .fetch_all(self.context.pool())
        .await?;
        let visual_summary: String = row.get("visual_summary");
        let user_note: String = row.get("user_note");
        let assets = asset_rows
            .into_iter()
            .map(|asset| memory_artifact(asset.get("id"), memory_id, &user_note))
            .collect::<Vec<_>>();
        let cover_id: Option<String> = row.get("cover_asset_id");
        let cover = cover_id.map(|id| memory_artifact(id, memory_id, &user_note));
        Ok(Some(MemorySearchResult {
            memory: MemoryRecord {
                id: row.get("id"),
                kind: row.get("kind"),
                user_note,
                visual_summary,
                captured_at: row.get("captured_at"),
                created_at: row.get("created_at"),
                cover,
                is_pinned: row.get::<i64, _>("is_pinned") != 0,
                archived_at: row.get("archived_at"),
            },
            score: 1.0,
            assets,
        }))
    }

    pub async fn update(&self, user_id: &str, memory_id: &str, note: &str) -> anyhow::Result<bool> {
        let note = note.trim();
        if note.is_empty() || note.chars().count() > 2_000 {
            anyhow::bail!("记忆内容不能为空且不能超过 2000 个字符");
        }
        let result = sqlx::query(
            "UPDATE memory_items SET user_note = $1, updated_at = $2 WHERE id = $3 AND user_id = $4",
        )
        .bind(note)
        .bind(unix_time())
        .bind(memory_id)
        .bind(user_id)
        .execute(self.context.pool())
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete(&self, user_id: &str, memory_id: &str) -> anyhow::Result<bool> {
        if self.get(user_id, memory_id).await?.is_none() {
            return Ok(false);
        }
        self.mutate(user_id, &[memory_id.to_owned()], LibraryAction::Delete)
            .await?;
        Ok(true)
    }

    pub async fn mutate(
        &self,
        user_id: &str,
        ids: &[String],
        action: LibraryAction,
    ) -> anyhow::Result<usize> {
        validate_library_ids(ids)?;
        let mut transaction = self.context.pool().begin().await?;
        let mut ownership =
            QueryBuilder::<Postgres>::new("SELECT COUNT(*) FROM memory_items WHERE user_id = ");
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
            anyhow::bail!("记忆不存在");
        }

        let mut orphaned = Vec::new();
        if action == LibraryAction::Delete {
            let mut assets = QueryBuilder::<Postgres>::new(
                "SELECT DISTINCT a.id, a.storage_key FROM memory_assets ma
                 JOIN assets a ON a.id = ma.asset_id
                 JOIN memory_items m ON m.id = ma.memory_id
                 WHERE m.user_id = ",
            );
            assets.push_bind(user_id).push(" AND ma.memory_id IN (");
            let mut separated = assets.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            let asset_rows = assets.build().fetch_all(&mut *transaction).await?;

            let mut detach =
                QueryBuilder::<Postgres>::new("DELETE FROM turn_attachments WHERE memory_id IN (");
            let mut separated = detach.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            detach.build().execute(&mut *transaction).await?;

            let mut delete =
                QueryBuilder::<Postgres>::new("DELETE FROM memory_items WHERE user_id = ");
            delete.push_bind(user_id).push(" AND id IN (");
            let mut separated = delete.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            delete.build().execute(&mut *transaction).await?;

            for row in asset_rows {
                let asset_id: String = row.get("id");
                let references: i64 = sqlx::query_scalar(
                    "SELECT
                        (SELECT COUNT(*) FROM memory_assets WHERE asset_id = $1) +
                        (SELECT COUNT(*) FROM memory_items WHERE cover_asset_id = $1) +
                        (SELECT COUNT(*) FROM turn_attachments WHERE asset_id = $1) +
                        (SELECT COUNT(*) FROM todos WHERE cover_asset_id = $1) +
                        (SELECT COUNT(*) FROM project_resources WHERE asset_id = $1) +
                        (SELECT COUNT(*) FROM library_resources WHERE asset_id = $1) +
                        (SELECT COUNT(*) FROM memory_evidence WHERE asset_id = $1) +
                        (SELECT COUNT(*) FROM users WHERE avatar_asset_id = $1)",
                )
                .bind(&asset_id)
                .fetch_one(&mut *transaction)
                .await?;
                if references == 0 {
                    sqlx::query("DELETE FROM assets WHERE id = $1 AND user_id = $2")
                        .bind(&asset_id)
                        .bind(user_id)
                        .execute(&mut *transaction)
                        .await?;
                    orphaned.push(row.get::<String, _>("storage_key"));
                }
            }
        } else {
            let mut update = QueryBuilder::<Postgres>::new("UPDATE memory_items SET ");
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
        transaction.commit().await?;
        for storage_key in orphaned {
            let _ = self.assets.remove(&storage_key).await;
        }
        Ok(ids.len())
    }

    pub async fn asset_content(
        &self,
        user_id: &str,
        asset_id: &str,
    ) -> anyhow::Result<Option<AssetContent>> {
        let row =
            sqlx::query("SELECT storage_key, mime_type FROM assets WHERE id = $1 AND user_id = $2")
                .bind(asset_id)
                .bind(user_id)
                .fetch_optional(self.context.pool())
                .await?;
        row.map(|row| {
            Ok(AssetContent {
                path: self.assets.resolve(&row.get::<String, _>("storage_key"))?,
                mime_type: row.get("mime_type"),
            })
        })
        .transpose()
    }

    pub async fn attach_to_turn(
        &self,
        turn_id: i64,
        artifacts: &[MemoryArtifact],
    ) -> anyhow::Result<()> {
        for (ordinal, item) in artifacts.iter().enumerate() {
            let memory_id = if item.todo_id.is_some() {
                None
            } else {
                Some(item.memory_id.as_str())
            };
            sqlx::query(
                "INSERT INTO turn_attachments(
                    turn_id, asset_id, memory_id, todo_id, caption, ordinal
                 ) VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(turn_id, asset_id) DO NOTHING",
            )
            .bind(turn_id)
            .bind(&item.id)
            .bind(memory_id)
            .bind(item.todo_id.as_deref())
            .bind(&item.caption)
            .bind(ordinal as i64)
            .execute(self.context.pool())
            .await?;
        }
        Ok(())
    }

    async fn get_todo(&self, user_id: &str, todo_id: &str) -> anyhow::Result<Option<TodoRecord>> {
        let row = sqlx::query(
            "SELECT id, title, visual_summary, cover_asset_id, due_at, completed_at, created_at
             FROM todos WHERE id = $1 AND user_id = $2",
        )
        .bind(todo_id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        let Some(row) = row else { return Ok(None) };
        let cover_id: Option<String> = row.get("cover_asset_id");
        let title: String = row.get("title");
        Ok(Some(TodoRecord {
            id: row.get("id"),
            memory_id: None,
            title: title.clone(),
            visual_summary: row.get("visual_summary"),
            due_at: row.get("due_at"),
            completed_at: row.get("completed_at"),
            created_at: row.get("created_at"),
            cover: cover_id.map(|asset_id| todo_artifact(asset_id, todo_id, &title)),
        }))
    }

    async fn existing_todo_execution(
        &self,
        response_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT todo_id FROM todo_tool_executions
             WHERE response_id = $1 AND tool_call_id = $2",
        )
        .bind(response_id)
        .bind(tool_call_id)
        .fetch_optional(self.context.pool())
        .await?)
    }

    async fn existing_execution(
        &self,
        response_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT memory_id FROM memory_tool_executions
             WHERE response_id = $1 AND tool_call_id = $2",
        )
        .bind(response_id)
        .bind(tool_call_id)
        .fetch_optional(self.context.pool())
        .await?)
    }
}

fn validate_library_ids(ids: &[String]) -> anyhow::Result<()> {
    if ids.is_empty() || ids.len() > 100 {
        anyhow::bail!("ids 必须包含 1 到 100 个项目");
    }
    let unique = ids.iter().collect::<HashSet<_>>();
    if unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty()) {
        anyhow::bail!("ids 不能包含空值或重复项");
    }
    Ok(())
}

fn memory_artifact(id: String, memory_id: &str, caption: &str) -> MemoryArtifact {
    MemoryArtifact {
        content_url: format!("/v1/assets/{id}/content"),
        id,
        kind: "image".to_owned(),
        memory_id: memory_id.to_owned(),
        todo_id: None,
        caption: caption.to_owned(),
    }
}

fn todo_artifact(id: String, todo_id: &str, caption: &str) -> MemoryArtifact {
    MemoryArtifact {
        content_url: format!("/v1/assets/{id}/content"),
        id,
        kind: "image".to_owned(),
        // Retain the existing response shape for mobile clients while giving
        // the persistence layer an explicit, correctly typed owner.
        memory_id: todo_id.to_owned(),
        todo_id: Some(todo_id.to_owned()),
        caption: caption.to_owned(),
    }
}

fn memory_fact_from_row(row: sqlx::postgres::PgRow) -> MemoryFactRecord {
    MemoryFactRecord {
        id: row.get("id"),
        kind: row.get("kind"),
        summary: row.get("summary"),
        scope_type: row.get("scope_type"),
        project_id: row.get("project_id"),
        source: row.get("source"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn validate_fact(kind: &str, summary: &str) -> anyhow::Result<()> {
    if !matches!(
        kind,
        "identity" | "preference" | "relationship" | "habit" | "context" | "other"
    ) {
        anyhow::bail!("不支持的记忆类别");
    }
    let length = summary.trim().chars().count();
    if length == 0 || length > 500 {
        anyhow::bail!("记忆内容不能为空且不能超过 500 个字符");
    }
    Ok(())
}

fn canonical_fact_key(summary: &str) -> String {
    summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn relevance(query: &str, candidate: &str) -> f64 {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return 1.0;
    }
    let candidate = candidate.to_lowercase();
    if candidate.contains(&normalized_query) {
        return 1.0;
    }
    let query_pairs = character_pairs(&normalized_query);
    if query_pairs.is_empty() {
        return f64::from(candidate.contains(&normalized_query));
    }
    let candidate_pairs = character_pairs(&candidate);
    let matches = query_pairs.intersection(&candidate_pairs).count();
    matches as f64 / query_pairs.len() as f64
}

fn character_pairs(value: &str) -> HashSet<String> {
    let characters = value
        .chars()
        .filter(|character| {
            !character.is_whitespace() && !"，。？！,.?!：:；;".contains(*character)
        })
        .collect::<Vec<_>>();
    characters
        .windows(2)
        .map(|pair| pair.iter().collect::<String>())
        .collect()
}

fn unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn ranks_related_chinese_text() {
        assert!(relevance("蓝色转接头放哪了", "蓝色转接头放在右边第二个抽屉") > 0.4);
        assert_eq!(relevance("蓝色转接头", "蓝色转接头放在抽屉"), 1.0);
        assert_eq!(relevance("咖啡", "蓝色转接头放在抽屉"), 0.0);
    }

    #[tokio::test]
    async fn creates_completes_lists_and_attaches_todo_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open_test().await.unwrap();
        context
            .seed_invitation_codes(&["todo-invite".to_owned()], 1, 24)
            .await
            .unwrap();
        let (user, _) = context
            .register_user("todo@example.com", "password-todo", "todo-invite", 24)
            .await
            .unwrap();
        let conversation = context.create_conversation(&user.id).await.unwrap();
        let turn = context
            .add_turn(&conversation, "user", "把这个做成待办", None)
            .await
            .unwrap();
        let service = MemoryService::new(context.clone(), directory.path().join("assets"))
            .await
            .unwrap();
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 3)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let request = CreateTodoRequest {
            user_id: user.id.clone(),
            conversation_id: conversation.clone(),
            source_turn_id: turn,
            response_id: "resp_todo".to_owned(),
            tool_call_id: "call_todo".to_owned(),
            title: "购买滤芯".to_owned(),
            visual_summary: "空调滤芯型号为 XX-123".to_owned(),
            due_at: Some(1_900_000_000.0),
            frames: vec![VideoFrame {
                bytes: jpeg.into_inner(),
                mime_type: "image/jpeg".to_owned(),
                captured_at_ms: Some(1_700_000_000_000),
                received_at_ms: 1_700_000_000_100,
            }],
        };
        let created = service.create_todo(request.clone()).await.unwrap();
        let repeated = service.create_todo(request).await.unwrap();
        assert_eq!(created.id, repeated.id);
        assert!(created.memory_id.is_none());
        let cover = created.cover.clone().unwrap();
        assert_eq!(cover.todo_id.as_deref(), Some(created.id.as_str()));
        let assistant_turn = context
            .add_turn(&conversation, "assistant", "已经创建待办", None)
            .await
            .unwrap();
        service
            .attach_to_turn(assistant_turn, std::slice::from_ref(&cover))
            .await
            .unwrap();
        let attachment =
            sqlx::query("SELECT memory_id, todo_id FROM turn_attachments WHERE turn_id = $1")
                .bind(assistant_turn)
                .fetch_one(context.pool())
                .await
                .unwrap();
        assert_eq!(attachment.get::<Option<String>, _>("memory_id"), None);
        assert_eq!(
            attachment.get::<Option<String>, _>("todo_id").as_deref(),
            Some(created.id.as_str())
        );
        let memory_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_items")
            .fetch_one(service.context.pool())
            .await
            .unwrap();
        assert_eq!(memory_count, 0);
        assert_eq!(
            service
                .list_todos(&user.id, Some(false), 10)
                .await
                .unwrap()
                .len(),
            1
        );
        let completed = service
            .complete_todo(&user.id, &created.id, true)
            .await
            .unwrap()
            .unwrap();
        assert!(completed.completed_at.is_some());
        assert!(
            service
                .list_todos(&user.id, Some(false), 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service
                .list_todos(&user.id, Some(true), 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn persists_recalls_attaches_and_deletes_visual_memory() {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open_test().await.unwrap();
        context
            .seed_invitation_codes(&["memory-invite".to_owned()], 2, 24)
            .await
            .unwrap();
        let (user, _) = context
            .register_user("memory@example.com", "password-memory", "memory-invite", 24)
            .await
            .unwrap();
        let (other, _) = context
            .register_user(
                "other-memory@example.com",
                "password-memory",
                "memory-invite",
                24,
            )
            .await
            .unwrap();
        let conversation = context.create_conversation(&user.id).await.unwrap();
        let user_turn_id = context
            .add_turn(&conversation, "user", "帮我记住蓝色转接头", None)
            .await
            .unwrap();
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 3)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let service = MemoryService::new(context.clone(), directory.path().join("assets"))
            .await
            .unwrap();
        let request = CreateMemoryRequest {
            user_id: user.id.clone(),
            conversation_id: conversation.clone(),
            source_turn_id: user_turn_id,
            response_id: "resp_1".to_owned(),
            tool_call_id: "call_1".to_owned(),
            user_note: "蓝色转接头放在右边第二个抽屉".to_owned(),
            visual_summary: "木柜右侧第二层抽屉里有蓝色转接头".to_owned(),
            frames: vec![VideoFrame {
                bytes: jpeg.into_inner(),
                mime_type: "image/jpeg".to_owned(),
                captured_at_ms: Some(1_700_000_000_000),
                received_at_ms: 1_700_000_000_100,
            }],
        };
        let created = service.create(request.clone()).await.unwrap();
        let repeated = service.create(request).await.unwrap();
        assert_eq!(created.memory.id, repeated.memory.id);
        let active = service
            .list(&user.id, LibraryScope::Active, false, "", 10)
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert!(!active[0].is_pinned);
        assert_eq!(active[0].archived_at, None);
        assert!(
            service
                .get(&other.id, &created.memory.id)
                .await
                .unwrap()
                .is_none()
        );

        let recalled = service
            .recall(&user.id, None, "蓝色转接头放哪了", 5)
            .await
            .unwrap();
        assert_eq!(recalled[0].memory.id, created.memory.id);

        service
            .mutate(
                &user.id,
                std::slice::from_ref(&created.memory.id),
                LibraryAction::Archive,
            )
            .await
            .unwrap();
        assert!(
            service
                .recall(&user.id, None, "蓝色转接头", 5)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            service
                .list(&user.id, LibraryScope::Archived, false, "", 10)
                .await
                .unwrap()
                .len(),
            1
        );
        service
            .mutate(
                &user.id,
                std::slice::from_ref(&created.memory.id),
                LibraryAction::Unarchive,
            )
            .await
            .unwrap();
        let cover = created.memory.cover.clone().unwrap();
        let assistant_turn = context
            .add_turn(&conversation, "assistant", "已经记住了", None)
            .await
            .unwrap();
        service
            .attach_to_turn(assistant_turn, std::slice::from_ref(&cover))
            .await
            .unwrap();
        let messages = context
            .conversation_messages(&user.id, &conversation, 20)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages.last().unwrap().attachments.len(), 1);
        assert!(
            service
                .asset_content(&other.id, &cover.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(service.delete(&user.id, &created.memory.id).await.unwrap());
        assert!(
            service
                .asset_content(&user.id, &cover.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn saved_memory_survives_its_conversation_and_batch_is_atomic() {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open_test().await.unwrap();
        context
            .seed_invitation_codes(
                &["survival-invite".to_owned(), "survival-other".to_owned()],
                1,
                24,
            )
            .await
            .unwrap();
        let (user, _) = context
            .register_user(
                "survival@example.com",
                "password-memory",
                "survival-invite",
                24,
            )
            .await
            .unwrap();
        let (other, _) = context
            .register_user(
                "survival-other@example.com",
                "password-memory",
                "survival-other",
                24,
            )
            .await
            .unwrap();
        let conversation = context.create_conversation(&user.id).await.unwrap();
        let source_turn_id = context
            .add_turn(&conversation, "user", "记住蓝色转接头", None)
            .await
            .unwrap();
        let other_conversation = context.create_conversation(&other.id).await.unwrap();
        let other_turn_id = context
            .add_turn(&other_conversation, "user", "记住红色转接头", None)
            .await
            .unwrap();
        let service = MemoryService::new(context.clone(), directory.path().join("assets"))
            .await
            .unwrap();
        let own = service
            .create(CreateMemoryRequest {
                user_id: user.id.clone(),
                conversation_id: conversation.clone(),
                source_turn_id,
                response_id: "survival-response".to_owned(),
                tool_call_id: "survival-call".to_owned(),
                user_note: "蓝色转接头".to_owned(),
                visual_summary: "蓝色转接头在抽屉里".to_owned(),
                frames: vec![],
            })
            .await
            .unwrap();
        let foreign = service
            .create(CreateMemoryRequest {
                user_id: other.id.clone(),
                conversation_id: other_conversation,
                source_turn_id: other_turn_id,
                response_id: "foreign-response".to_owned(),
                tool_call_id: "foreign-call".to_owned(),
                user_note: "红色转接头".to_owned(),
                visual_summary: "红色转接头在桌上".to_owned(),
                frames: vec![],
            })
            .await
            .unwrap();

        assert!(
            service
                .mutate(
                    &user.id,
                    &[own.memory.id.clone(), foreign.memory.id],
                    LibraryAction::Pin,
                )
                .await
                .is_err()
        );
        assert!(
            !service
                .get(&user.id, &own.memory.id)
                .await
                .unwrap()
                .unwrap()
                .memory
                .is_pinned
        );

        context
            .mutate_conversations(
                &user.id,
                std::slice::from_ref(&conversation),
                LibraryAction::Delete,
            )
            .await
            .unwrap();
        assert!(
            service
                .get(&user.id, &own.memory.id)
                .await
                .unwrap()
                .is_some()
        );
        let source: (Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT conversation_id, source_turn_id FROM memory_items WHERE id = $1",
        )
        .bind(&own.memory.id)
        .fetch_one(context.pool())
        .await
        .unwrap();
        assert_eq!(source, (None, None));
    }

    #[tokio::test]
    async fn project_memories_are_recalled_only_inside_their_project() {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open_test().await.unwrap();
        context
            .seed_invitation_codes(&["project-memory-invite".to_owned()], 1, 24)
            .await
            .unwrap();
        let (user, _) = context
            .register_user(
                "project-memory@example.com",
                "password-project-memory",
                "project-memory-invite",
                24,
            )
            .await
            .unwrap();
        let personal_conversation = context.create_conversation(&user.id).await.unwrap();
        let personal_turn = context
            .add_turn(&personal_conversation, "user", "我喜欢乌龙茶", None)
            .await
            .unwrap();
        let project = context
            .create_project(&user.id, "数据库迁移", "", "只使用 Responses API")
            .await
            .unwrap();
        let project_conversation = context
            .create_project_conversation(&user.id, &project.id)
            .await
            .unwrap();
        let project_turn = context
            .add_turn(
                &project_conversation,
                "user",
                "项目数据库确定使用 PostgreSQL",
                None,
            )
            .await
            .unwrap();
        let service = MemoryService::new(context.clone(), directory.path().join("assets"))
            .await
            .unwrap();
        service
            .create(CreateMemoryRequest {
                user_id: user.id.clone(),
                conversation_id: personal_conversation.clone(),
                source_turn_id: personal_turn,
                response_id: "personal-memory-response".to_owned(),
                tool_call_id: "personal-memory-call".to_owned(),
                user_note: "用户喜欢乌龙茶".to_owned(),
                visual_summary: String::new(),
                frames: vec![],
            })
            .await
            .unwrap();
        service
            .create(CreateMemoryRequest {
                user_id: user.id.clone(),
                conversation_id: project_conversation.clone(),
                source_turn_id: project_turn,
                response_id: "project-memory-response".to_owned(),
                tool_call_id: "project-memory-call".to_owned(),
                user_note: "项目数据库使用 PostgreSQL".to_owned(),
                visual_summary: String::new(),
                frames: vec![],
            })
            .await
            .unwrap();

        assert!(
            service
                .recall(&user.id, None, "PostgreSQL", 5)
                .await
                .unwrap()
                .is_empty()
        );
        let project_results = service
            .recall(
                &user.id,
                Some(&project_conversation),
                "PostgreSQL 乌龙茶",
                5,
            )
            .await
            .unwrap();
        assert_eq!(project_results.len(), 2);
    }

    #[test]
    fn fact_validation_keeps_categories_and_content_bounded() {
        assert!(validate_fact("preference", "用户喜欢先听结论").is_ok());
        assert!(validate_fact("unknown", "用户喜欢先听结论").is_err());
        assert!(validate_fact("other", "   ").is_err());
        assert!(validate_fact("other", &"记".repeat(501)).is_err());
    }

    #[test]
    fn canonical_fact_keys_ignore_outer_and_repeated_whitespace() {
        assert_eq!(
            canonical_fact_key("  Likes   Oolong Tea "),
            "likes oolong tea"
        );
    }
}
