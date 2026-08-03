use std::{cmp::Ordering, collections::HashSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row, Sqlite};
use uuid::Uuid;

use crate::{
    asset_store::AssetStore,
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
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            let row = sqlx::query("SELECT id FROM assets WHERE user_id = ? AND sha256 = ?")
                .bind(&request.user_id)
                .bind(&asset.sha256)
                .fetch_one(&mut *transaction)
                .await?;
            asset_ids.push(row.get::<String, _>("id"));
        }
        let cover_asset_id = asset_ids.last().cloned();
        sqlx::query(
            "INSERT INTO memory_items(
                id, user_id, conversation_id, source_turn_id, source_response_id,
                kind, user_note, visual_summary, cover_asset_id, captured_at,
                created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&memory_id)
        .bind(&request.user_id)
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
                 VALUES (?, ?, ?, ?)",
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
             VALUES (?, ?, ?)",
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
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemorySearchResult>> {
        let rows = sqlx::query(
            "SELECT id FROM memory_items WHERE user_id = ? AND archived_at IS NULL
             ORDER BY created_at DESC LIMIT 200",
        )
        .bind(user_id)
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
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                    "SELECT id FROM assets WHERE user_id = ? AND sha256 = ?",
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
             ) VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)",
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
             VALUES (?, ?, ?)",
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
                "SELECT id FROM todos WHERE user_id = ? AND completed_at IS NOT NULL
                 ORDER BY completed_at DESC LIMIT ?",
            )
            .bind(user_id)
            .bind(limit.clamp(1, 100) as i64)
            .fetch_all(self.context.pool())
            .await?,
            _ => sqlx::query(
                "SELECT id FROM todos WHERE user_id = ? AND completed_at IS NULL
                 ORDER BY CASE WHEN due_at IS NULL THEN 1 ELSE 0 END, due_at ASC, created_at DESC LIMIT ?",
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

    pub async fn complete_todo(
        &self,
        user_id: &str,
        todo_id: &str,
        completed: bool,
    ) -> anyhow::Result<Option<TodoRecord>> {
        let now = unix_time();
        let result = sqlx::query(
            "UPDATE todos SET completed_at = ?, updated_at = ? WHERE id = ? AND user_id = ?",
        )
        .bind(if completed { Some(now) } else { None })
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
            "SELECT id FROM memory_items WHERE user_id = ?
               AND (? = 2 OR (? = 0 AND archived_at IS NULL)
                    OR (? = 1 AND archived_at IS NOT NULL))
               AND (? = 0 OR is_pinned = 1)
               AND (? = '' OR user_note LIKE ? OR visual_summary LIKE ?)
             ORDER BY is_pinned DESC, COALESCE(captured_at, created_at) DESC LIMIT ?",
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
             FROM memory_items WHERE id = ? AND user_id = ?",
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
             WHERE ma.memory_id = ? AND a.user_id = ? ORDER BY ma.ordinal ASC",
        )
        .bind(memory_id)
        .bind(user_id)
        .fetch_all(self.context.pool())
        .await?;
        let visual_summary: String = row.get("visual_summary");
        let user_note: String = row.get("user_note");
        let assets = asset_rows
            .into_iter()
            .map(|asset| artifact(asset.get("id"), memory_id, &user_note))
            .collect::<Vec<_>>();
        let cover_id: Option<String> = row.get("cover_asset_id");
        let cover = cover_id.map(|id| artifact(id, memory_id, &user_note));
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
            "UPDATE memory_items SET user_note = ?, updated_at = ? WHERE id = ? AND user_id = ?",
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
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM memory_items WHERE user_id = ");
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
            let mut assets = QueryBuilder::<Sqlite>::new(
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
                QueryBuilder::<Sqlite>::new("DELETE FROM turn_attachments WHERE memory_id IN (");
            let mut separated = detach.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            detach.build().execute(&mut *transaction).await?;

            let mut delete =
                QueryBuilder::<Sqlite>::new("DELETE FROM memory_items WHERE user_id = ");
            delete.push_bind(user_id).push(" AND id IN (");
            let mut separated = delete.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            delete.build().execute(&mut *transaction).await?;

            for row in asset_rows {
                let asset_id: String = row.get("id");
                let references: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM memory_assets WHERE asset_id = ?")
                        .bind(&asset_id)
                        .fetch_one(&mut *transaction)
                        .await?;
                if references == 0 {
                    sqlx::query("DELETE FROM assets WHERE id = ? AND user_id = ?")
                        .bind(&asset_id)
                        .bind(user_id)
                        .execute(&mut *transaction)
                        .await?;
                    orphaned.push(row.get::<String, _>("storage_key"));
                }
            }
        } else {
            let mut update = QueryBuilder::<Sqlite>::new("UPDATE memory_items SET ");
            match action {
                LibraryAction::Pin => update.push("is_pinned = 1"),
                LibraryAction::Unpin => update.push("is_pinned = 0"),
                LibraryAction::Archive => {
                    update.push("archived_at = COALESCE(archived_at, ");
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
            sqlx::query("SELECT storage_key, mime_type FROM assets WHERE id = ? AND user_id = ?")
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
            sqlx::query(
                "INSERT OR IGNORE INTO turn_attachments(
                    turn_id, asset_id, memory_id, caption, ordinal
                 ) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(turn_id)
            .bind(&item.id)
            .bind(&item.memory_id)
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
             FROM todos WHERE id = ? AND user_id = ?",
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
            cover: cover_id.map(|asset_id| artifact(asset_id, todo_id, &title)),
        }))
    }

    async fn existing_todo_execution(
        &self,
        response_id: &str,
        tool_call_id: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT todo_id FROM todo_tool_executions
             WHERE response_id = ? AND tool_call_id = ?",
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
             WHERE response_id = ? AND tool_call_id = ?",
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

fn artifact(id: String, memory_id: &str, caption: &str) -> MemoryArtifact {
    MemoryArtifact {
        content_url: format!("/v1/assets/{id}/content"),
        id,
        kind: "image".to_owned(),
        memory_id: memory_id.to_owned(),
        caption: caption.to_owned(),
    }
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
    async fn creates_completes_and_lists_todo_with_memory_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let context = ContextStore::open(&directory.path().join("context.sqlite3"))
            .await
            .unwrap();
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
        let service = MemoryService::new(context, directory.path().join("assets"))
            .await
            .unwrap();
        let request = CreateTodoRequest {
            user_id: user.id.clone(),
            conversation_id: conversation,
            source_turn_id: turn,
            response_id: "resp_todo".to_owned(),
            tool_call_id: "call_todo".to_owned(),
            title: "购买滤芯".to_owned(),
            visual_summary: "空调滤芯型号为 XX-123".to_owned(),
            due_at: Some(1_900_000_000.0),
            frames: Vec::new(),
        };
        let created = service.create_todo(request.clone()).await.unwrap();
        let repeated = service.create_todo(request).await.unwrap();
        assert_eq!(created.id, repeated.id);
        assert!(created.memory_id.is_none());
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
        let context = ContextStore::open(&directory.path().join("context.sqlite3"))
            .await
            .unwrap();
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
            .recall(&user.id, "蓝色转接头放哪了", 5)
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
                .recall(&user.id, "蓝色转接头", 5)
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
        let context = ContextStore::open(&directory.path().join("survival.sqlite3"))
            .await
            .unwrap();
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
        let source: (Option<String>, Option<i64>) =
            sqlx::query_as("SELECT conversation_id, source_turn_id FROM memory_items WHERE id = ?")
                .bind(&own.memory.id)
                .fetch_one(context.pool())
                .await
                .unwrap();
        assert_eq!(source, (None, None));
    }
}
