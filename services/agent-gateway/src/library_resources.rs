use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    asset_store::{AssetStore, StoredAsset},
    context::ContextStore,
};

const MAX_TITLE_CHARS: usize = 300;
const MAX_TEXT_CHARS: usize = 200_000;

#[derive(Clone)]
pub struct LibraryResourceService {
    context: ContextStore,
    assets: AssetStore,
}

#[derive(Clone, Debug, Serialize)]
pub struct LibraryResourceRecord {
    pub id: String,
    pub title: String,
    pub resource_type: String,
    pub scope_type: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub source_url: Option<String>,
    pub mime_type: Option<String>,
    pub asset_id: Option<String>,
    pub content: String,
    pub metadata: Value,
    pub status: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub archived_at: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct CreateLibraryResource {
    pub resource_type: String,
    pub title: String,
    pub content: Option<String>,
    pub source_url: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub data_base64: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateLibraryResource {
    pub title: Option<String>,
    pub content: Option<String>,
    pub source_url: Option<String>,
    pub archived: Option<bool>,
}

impl LibraryResourceService {
    pub async fn new(context: ContextStore, root: std::path::PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            context,
            assets: AssetStore::new(root).await?,
        })
    }

    pub async fn list(
        &self,
        user_id: &str,
        query: &str,
        resource_type: Option<&str>,
        archived: bool,
        limit: usize,
    ) -> anyhow::Result<Vec<LibraryResourceRecord>> {
        if let Some(resource_type) = resource_type {
            validate_resource_type(resource_type)?;
        }
        let query = query.trim();
        let rows = sqlx::query(
            "SELECT r.id, r.title, r.resource_type, r.scope_type, r.project_id,
                    p.name AS project_name, r.source_url, r.mime_type, r.asset_id,
                    LEFT(r.extracted_text, 400) AS content, r.metadata, r.status,
                    r.created_at, r.updated_at, r.archived_at
             FROM library_resources r
             LEFT JOIN projects p ON p.id = r.project_id AND p.owner_user_id = r.user_id
             WHERE r.user_id = $1
               AND (($2 = TRUE AND r.archived_at IS NOT NULL)
                    OR ($2 = FALSE AND r.archived_at IS NULL))
               AND ($3::TEXT IS NULL OR r.resource_type = $3)
               AND ($4 = '' OR r.title ILIKE '%' || $4 || '%'
                    OR r.extracted_text ILIKE '%' || $4 || '%'
                    OR COALESCE(r.source_url, '') ILIKE '%' || $4 || '%')
             ORDER BY r.updated_at DESC LIMIT $5",
        )
        .bind(user_id)
        .bind(archived)
        .bind(resource_type)
        .bind(query)
        .bind(limit.clamp(1, 100) as i64)
        .fetch_all(self.context.pool())
        .await?;
        Ok(rows.into_iter().map(resource_from_row).collect())
    }

    pub async fn get(
        &self,
        user_id: &str,
        resource_id: &str,
    ) -> anyhow::Result<Option<LibraryResourceRecord>> {
        let row = sqlx::query(
            "SELECT r.id, r.title, r.resource_type, r.scope_type, r.project_id,
                    p.name AS project_name, r.source_url, r.mime_type, r.asset_id,
                    r.extracted_text AS content, r.metadata, r.status,
                    r.created_at, r.updated_at, r.archived_at
             FROM library_resources r
             LEFT JOIN projects p ON p.id = r.project_id AND p.owner_user_id = r.user_id
             WHERE r.id = $1 AND r.user_id = $2",
        )
        .bind(resource_id)
        .bind(user_id)
        .fetch_optional(self.context.pool())
        .await?;
        Ok(row.map(resource_from_row))
    }

    pub async fn create(
        &self,
        user_id: &str,
        request: CreateLibraryResource,
    ) -> anyhow::Result<LibraryResourceRecord> {
        validate_resource_type(&request.resource_type)?;
        let title = validate_title(&request.title)?;
        let project_id = request
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        if let Some(project_id) = project_id {
            self.ensure_project(user_id, project_id).await?;
        }
        let scope_type = if project_id.is_some() {
            "project"
        } else {
            "personal"
        };
        let mut source_url = None;
        let mut extracted_text = String::new();
        let mut stored_asset = None;
        let mut metadata = json!({});
        let status;

        match request.resource_type.as_str() {
            "note" => {
                extracted_text = validate_text(request.content.as_deref().unwrap_or_default())?;
                if extracted_text.is_empty() {
                    anyhow::bail!("笔记内容不能为空");
                }
                status = "ready";
            }
            "link" => {
                let url = validate_url(request.source_url.as_deref().unwrap_or_default())?;
                source_url = Some(url.clone());
                extracted_text = request
                    .content
                    .as_deref()
                    .map(validate_text)
                    .transpose()?
                    .unwrap_or_default();
                if extracted_text.is_empty() {
                    extracted_text = format!("{title}\n{url}");
                }
                status = "ready";
            }
            "file" => {
                let mime_type = request.mime_type.as_deref().unwrap_or_default();
                let encoded = request.data_base64.as_deref().unwrap_or_default();
                let bytes = STANDARD
                    .decode(encoded)
                    .map_err(|_| anyhow::anyhow!("文件内容编码无效"))?;
                let stored = self
                    .assets
                    .store_document(user_id, mime_type, &bytes)
                    .await?;
                if stored.mime_type.starts_with("text/") {
                    extracted_text = validate_text(
                        std::str::from_utf8(&bytes)
                            .map_err(|_| anyhow::anyhow!("文本文件必须使用 UTF-8 编码"))?,
                    )?;
                    status = "ready";
                } else {
                    status = "stored";
                }
                metadata = json!({
                    "file_name": request.file_name.as_deref().unwrap_or(&request.title),
                    "size_bytes": bytes.len()
                });
                stored_asset = Some(stored);
            }
            _ => unreachable!(),
        }

        let resource_id = format!("resource_{}", Uuid::new_v4().simple());
        let now = unix_time();
        let mut transaction = self.context.pool().begin().await?;
        let asset_id = if let Some(asset) = stored_asset.as_ref() {
            Some(insert_asset(&mut transaction, user_id, asset, now).await?)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO library_resources(
                id, user_id, scope_type, project_id, title, resource_type,
                asset_id, source_url, mime_type, extracted_text, metadata,
                status, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)",
        )
        .bind(&resource_id)
        .bind(user_id)
        .bind(scope_type)
        .bind(project_id)
        .bind(title)
        .bind(&request.resource_type)
        .bind(&asset_id)
        .bind(&source_url)
        .bind(stored_asset.as_ref().map(|asset| asset.mime_type.as_str()))
        .bind(&extracted_text)
        .bind(&metadata)
        .bind(status)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sync_retrieval(
            &mut transaction,
            user_id,
            scope_type,
            project_id,
            &resource_id,
            title,
            &extracted_text,
            now,
        )
        .await?;
        transaction.commit().await?;
        self.get(user_id, &resource_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("创建后的资料不存在"))
    }

    pub async fn update(
        &self,
        user_id: &str,
        resource_id: &str,
        patch: UpdateLibraryResource,
    ) -> anyhow::Result<Option<LibraryResourceRecord>> {
        let Some(current) = self.get(user_id, resource_id).await? else {
            return Ok(None);
        };
        if patch.title.is_none()
            && patch.content.is_none()
            && patch.source_url.is_none()
            && patch.archived.is_none()
        {
            anyhow::bail!("至少提供一项资料修改内容");
        }
        let title = patch
            .title
            .as_deref()
            .map(validate_title)
            .transpose()?
            .unwrap_or(&current.title);
        let content = if let Some(content) = patch.content.as_deref() {
            if current.resource_type == "file" {
                anyhow::bail!("文件正文不能直接编辑");
            }
            validate_text(content)?
        } else {
            current.content.clone()
        };
        if current.resource_type == "note" && content.is_empty() {
            anyhow::bail!("笔记内容不能为空");
        }
        let source_url = if let Some(url) = patch.source_url.as_deref() {
            if current.resource_type != "link" {
                anyhow::bail!("只有网页资料可以修改链接");
            }
            Some(validate_url(url)?)
        } else {
            current.source_url.clone()
        };
        let now = unix_time();
        let archived_at = patch
            .archived
            .map(|archived| if archived { Some(now) } else { None });
        let mut transaction = self.context.pool().begin().await?;
        sqlx::query(
            "UPDATE library_resources SET title = $1, extracted_text = $2,
                    source_url = $3,
                    archived_at = CASE WHEN $4 THEN $5 ELSE archived_at END,
                    updated_at = $6
             WHERE id = $7 AND user_id = $8",
        )
        .bind(title)
        .bind(&content)
        .bind(&source_url)
        .bind(archived_at.is_some())
        .bind(archived_at.flatten())
        .bind(now)
        .bind(resource_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
        sync_retrieval(
            &mut transaction,
            user_id,
            &current.scope_type,
            current.project_id.as_deref(),
            resource_id,
            title,
            &content,
            now,
        )
        .await?;
        transaction.commit().await?;
        self.get(user_id, resource_id).await
    }

    pub async fn delete(&self, user_id: &str, resource_id: &str) -> anyhow::Result<bool> {
        let Some(current) = self.get(user_id, resource_id).await? else {
            return Ok(false);
        };
        let mut transaction = self.context.pool().begin().await?;
        sqlx::query("DELETE FROM retrieval_documents WHERE source_type = 'library_resource' AND source_id = $1 AND user_id = $2")
            .bind(resource_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM library_resources WHERE id = $1 AND user_id = $2")
            .bind(resource_id)
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        let mut orphaned_storage_key = None;
        if let Some(asset_id) = current.asset_id.as_deref() {
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
            .fetch_one(&mut *transaction)
            .await?;
            if references == 0 {
                orphaned_storage_key = sqlx::query_scalar::<_, String>(
                    "DELETE FROM assets WHERE id = $1 AND user_id = $2 RETURNING storage_key",
                )
                .bind(asset_id)
                .bind(user_id)
                .fetch_optional(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        if let Some(storage_key) = orphaned_storage_key {
            let _ = self.assets.remove(&storage_key).await;
        }
        Ok(true)
    }

    async fn ensure_project(&self, user_id: &str, project_id: &str) -> anyhow::Result<()> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND owner_user_id = $2 AND archived_at IS NULL)",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_one(self.context.pool())
        .await?;
        if !exists {
            anyhow::bail!("项目不存在或已归档");
        }
        Ok(())
    }
}

async fn insert_asset(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &str,
    asset: &StoredAsset,
    now: f64,
) -> anyhow::Result<String> {
    sqlx::query(
        "INSERT INTO assets(
            id, user_id, sha256, mime_type, storage_key, width, height,
            size_bytes, captured_at, created_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9)
         ON CONFLICT(user_id, sha256) DO NOTHING",
    )
    .bind(&asset.id)
    .bind(user_id)
    .bind(&asset.sha256)
    .bind(&asset.mime_type)
    .bind(&asset.storage_key)
    .bind(i64::from(asset.width))
    .bind(i64::from(asset.height))
    .bind(asset.size_bytes as i64)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    Ok(
        sqlx::query_scalar::<_, String>("SELECT id FROM assets WHERE user_id = $1 AND sha256 = $2")
            .bind(user_id)
            .bind(&asset.sha256)
            .fetch_one(&mut **transaction)
            .await?,
    )
}

#[allow(clippy::too_many_arguments)]
async fn sync_retrieval(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: &str,
    scope_type: &str,
    project_id: Option<&str>,
    resource_id: &str,
    title: &str,
    content: &str,
    now: f64,
) -> anyhow::Result<()> {
    let document_id = format!("document_{resource_id}");
    sqlx::query(
        "INSERT INTO retrieval_documents(
            id, user_id, scope_type, project_id, source_type, source_id,
            title, status, occurred_at, created_at
         ) VALUES ($1, $2, $3, $4, 'library_resource', $5, $6, 'active', $7, $7)
         ON CONFLICT(source_type, source_id) DO UPDATE SET
            scope_type = EXCLUDED.scope_type, project_id = EXCLUDED.project_id,
            title = EXCLUDED.title, status = 'active'",
    )
    .bind(&document_id)
    .bind(user_id)
    .bind(scope_type)
    .bind(project_id)
    .bind(resource_id)
    .bind(title)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("DELETE FROM retrieval_chunks WHERE document_id = $1")
        .bind(&document_id)
        .execute(&mut **transaction)
        .await?;
    for (ordinal, chunk) in chunk_text(content).into_iter().enumerate() {
        let hash = format!("{:x}", sha2::Sha256::digest(chunk.as_bytes()));
        sqlx::query(
            "INSERT INTO retrieval_chunks(
                id, document_id, ordinal, content, content_hash, importance, created_at
             ) VALUES ($1, $2, $3, $4, $5, 0.5, $6)",
        )
        .bind(format!("chunk_{}", Uuid::new_v4().simple()))
        .bind(&document_id)
        .bind(ordinal as i64)
        .bind(chunk)
        .bind(hash)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn resource_from_row(row: sqlx::postgres::PgRow) -> LibraryResourceRecord {
    LibraryResourceRecord {
        id: row.get("id"),
        title: row.get("title"),
        resource_type: row.get("resource_type"),
        scope_type: row.get("scope_type"),
        project_id: row.get("project_id"),
        project_name: row.get("project_name"),
        source_url: row.get("source_url"),
        mime_type: row.get("mime_type"),
        asset_id: row.get("asset_id"),
        content: row.get("content"),
        metadata: row.get("metadata"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        archived_at: row.get("archived_at"),
    }
}

fn validate_resource_type(resource_type: &str) -> anyhow::Result<()> {
    if matches!(resource_type, "note" | "link" | "file") {
        Ok(())
    } else {
        anyhow::bail!("资料类型无效")
    }
}

fn validate_title(title: &str) -> anyhow::Result<&str> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        anyhow::bail!("资料标题不能为空且不能超过 300 个字符");
    }
    Ok(title)
}

fn validate_text(content: &str) -> anyhow::Result<String> {
    let content = content.trim();
    if content.chars().count() > MAX_TEXT_CHARS {
        anyhow::bail!("资料正文不能超过 20 万个字符");
    }
    Ok(content.to_owned())
}

fn validate_url(value: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| anyhow::anyhow!("网页链接无效"))?;
    if !matches!(url.scheme(), "http" | "https") || !url.has_host() || !url.username().is_empty() {
        anyhow::bail!("网页链接必须是有效的 HTTP 或 HTTPS 地址");
    }
    Ok(url.to_string())
}

fn chunk_text(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in content
        .split('\n')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if current.chars().count() + paragraph.chars().count() + 1 > 1_000 && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if paragraph.chars().count() > 1_000 {
            for character in paragraph.chars() {
                current.push(character);
                if current.chars().count() == 1_000 {
                    chunks.push(std::mem::take(&mut current));
                }
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(paragraph);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
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

    #[test]
    fn chunks_long_unicode_text_without_losing_content() {
        let content = "资料".repeat(1_200);
        let chunks = chunk_text(&content);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.concat(), content);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 1_000));
    }

    #[test]
    fn validates_only_safe_resource_urls() {
        assert!(validate_url("https://example.com/path").is_ok());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user@example.com/").is_err());
    }
}
