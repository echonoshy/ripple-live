use std::path::{Path, PathBuf};

use image::GenericImageView;
use sha2::{Digest, Sha256};
use tokio::fs;
use uuid::Uuid;

const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 4_096;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;

#[derive(Clone)]
pub struct AssetStore {
    root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct StoredAsset {
    pub id: String,
    pub sha256: String,
    pub mime_type: String,
    pub storage_key: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: usize,
}

impl AssetStore {
    pub async fn new(root: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    pub async fn store_jpeg(&self, user_id: &str, bytes: &[u8]) -> anyhow::Result<StoredAsset> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            anyhow::bail!("图片大小必须在 1 字节到 2MB 之间");
        }
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
            .map_err(|_| anyhow::anyhow!("无法解析 JPEG 图片"))?;
        let (width, height) = image.dimensions();
        if width == 0
            || height == 0
            || width > MAX_IMAGE_SIDE
            || height > MAX_IMAGE_SIDE
            || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
        {
            anyhow::bail!("图片尺寸超出限制");
        }

        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let safe_user = safe_segment(user_id)?;
        let relative = format!("{safe_user}/{sha256}.jpg");
        let final_path = self.root.join(&relative);
        if fs::metadata(&final_path).await.is_err() {
            let user_dir = self.root.join(&safe_user);
            fs::create_dir_all(&user_dir).await?;
            let temporary = user_dir.join(format!(".{}.tmp", Uuid::new_v4().simple()));
            fs::write(&temporary, bytes).await?;
            if fs::metadata(&final_path).await.is_ok() {
                let _ = fs::remove_file(&temporary).await;
            } else if let Err(error) = fs::rename(&temporary, &final_path).await {
                let _ = fs::remove_file(&temporary).await;
                return Err(error.into());
            }
        }

        Ok(StoredAsset {
            id: format!("asset_{}", Uuid::new_v4().simple()),
            sha256,
            mime_type: "image/jpeg".to_owned(),
            storage_key: relative,
            width,
            height,
            size_bytes: bytes.len(),
        })
    }

    pub fn resolve(&self, storage_key: &str) -> anyhow::Result<PathBuf> {
        let path = Path::new(storage_key);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            anyhow::bail!("无效的资产路径");
        }
        Ok(self.root.join(path))
    }

    pub async fn remove(&self, storage_key: &str) -> anyhow::Result<()> {
        let path = self.resolve(storage_key)?;
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn safe_segment(value: &str) -> anyhow::Result<String> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        anyhow::bail!("无效的用户标识");
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_non_jpeg_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let store = AssetStore::new(temp.path().to_owned()).await.unwrap();
        assert!(store.store_jpeg("user_1", b"not an image").await.is_err());
    }

    #[test]
    fn rejects_unsafe_user_segments() {
        assert!(safe_segment("../other").is_err());
        assert_eq!(safe_segment("user_123").unwrap(), "user_123");
    }
}
