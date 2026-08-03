use std::{cmp::Ordering, collections::HashSet, path::PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{asset_store::AssetStore, context::ContextStore, protocol::VideoFrame};

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
}

#[derive(Clone, Debug, Serialize)]
pub struct MemorySearchResult {
    #[serde(flatten)]
    pub memory: MemoryRecord,
    pub score: f64,
    pub assets: Vec<MemoryArtifact>,
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
            "SELECT id FROM memory_items WHERE user_id = ? ORDER BY created_at DESC LIMIT 200",
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

    pub async fn list(&self, user_id: &str, limit: usize) -> anyhow::Result<Vec<MemoryRecord>> {
        let rows = sqlx::query(
            "SELECT id FROM memory_items WHERE user_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(user_id)
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
                    captured_at, created_at
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
        let asset_rows = sqlx::query(
            "SELECT a.id, a.storage_key FROM memory_assets ma
             JOIN assets a ON a.id = ma.asset_id
             JOIN memory_items m ON m.id = ma.memory_id
             WHERE ma.memory_id = ? AND m.user_id = ?",
        )
        .bind(memory_id)
        .bind(user_id)
        .fetch_all(self.context.pool())
        .await?;
        let mut transaction = self.context.pool().begin().await?;
        sqlx::query("DELETE FROM turn_attachments WHERE memory_id = ?")
            .bind(memory_id)
            .execute(&mut *transaction)
            .await?;
        let deleted = sqlx::query("DELETE FROM memory_items WHERE id = ? AND user_id = ?")
            .bind(memory_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            == 1;
        let mut orphaned = Vec::new();
        if deleted {
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
        }
        transaction.commit().await?;
        for storage_key in orphaned {
            let _ = self.assets.remove(&storage_key).await;
        }
        Ok(deleted)
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
        assert_eq!(service.list(&user.id, 10).await.unwrap().len(), 1);
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
}
