use std::{
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Mutex,
};
use uuid::Uuid;

use super::types::FinalAudioMetadata;

#[derive(Clone)]
pub struct MeetingStorage {
    root: PathBuf,
    maximum_chunk_bytes: u64,
    ffmpeg_bin: PathBuf,
    mutation_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutChunkOutcome {
    Inserted,
    Existing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredChunk {
    pub outcome: PutChunkOutcome,
    pub relative_path: String,
    pub size_bytes: u64,
    pub checksum: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalAudio {
    pub relative_path: String,
    pub size_bytes: u64,
    pub checksum: String,
}

#[derive(Debug)]
pub struct VerifiedLegacyFinalization {
    pub(super) meeting_id: String,
    pub(super) last_sequence: i64,
    pub(super) audio: FinalAudioMetadata,
}

#[derive(Debug)]
pub enum StorageError {
    UnsafePath,
    InvalidChecksum,
    TooLarge,
    ChecksumMismatch,
    Conflict,
    Io(std::io::Error),
    Stream(String),
    Ffmpeg(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafePath => write!(formatter, "unsafe meeting storage path"),
            Self::InvalidChecksum => write!(formatter, "invalid SHA-256 checksum"),
            Self::TooLarge => write!(formatter, "meeting chunk exceeds configured size limit"),
            Self::ChecksumMismatch => write!(formatter, "meeting chunk checksum mismatch"),
            Self::Conflict => write!(formatter, "immutable meeting chunk conflicts"),
            Self::Io(error) => write!(formatter, "meeting storage I/O failed: {error}"),
            Self::Stream(error) => write!(formatter, "meeting upload stream failed: {error}"),
            Self::Ffmpeg(error) => write!(formatter, "meeting FFmpeg operation failed: {error}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl MeetingStorage {
    pub fn new(root: PathBuf, maximum_chunk_bytes: u64, ffmpeg_bin: PathBuf) -> Self {
        Self {
            root,
            maximum_chunk_bytes,
            ffmpeg_bin,
            mutation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn put_chunk<S, E>(
        &self,
        meeting_id: &str,
        sequence: i64,
        expected_checksum: &str,
        mut body: S,
    ) -> Result<StoredChunk, StorageError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        validate_meeting_id(meeting_id)?;
        if sequence < 0 {
            return Err(StorageError::UnsafePath);
        }
        let expected_checksum = normalize_checksum(expected_checksum)?;
        let relative_path = format!("{meeting_id}/chunks/{sequence}.m4a");
        let final_path = self.resolve_relative(&relative_path)?;
        let chunk_directory = final_path.parent().ok_or(StorageError::UnsafePath)?;
        fs::create_dir_all(chunk_directory).await?;
        let temporary_path =
            chunk_directory.join(format!(".{sequence}.{}.tmp", Uuid::new_v4().hyphenated()));
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .await?;
        let mut hasher = Sha256::new();
        let mut size_bytes = 0_u64;

        while let Some(item) = body.next().await {
            let bytes = match item {
                Ok(bytes) => bytes,
                Err(error) => {
                    drop(temporary);
                    remove_if_present(&temporary_path).await;
                    return Err(StorageError::Stream(error.to_string()));
                }
            };
            size_bytes = match size_bytes.checked_add(bytes.len() as u64) {
                Some(size_bytes) => size_bytes,
                None => {
                    drop(temporary);
                    remove_if_present(&temporary_path).await;
                    return Err(StorageError::TooLarge);
                }
            };
            if size_bytes > self.maximum_chunk_bytes {
                drop(temporary);
                remove_if_present(&temporary_path).await;
                return Err(StorageError::TooLarge);
            }
            if let Err(error) = temporary.write_all(&bytes).await {
                drop(temporary);
                remove_if_present(&temporary_path).await;
                return Err(error.into());
            }
            hasher.update(&bytes);
        }
        if let Err(error) = temporary.sync_all().await {
            drop(temporary);
            remove_if_present(&temporary_path).await;
            return Err(error.into());
        }
        drop(temporary);

        let checksum = format!("{:x}", hasher.finalize());
        if checksum != expected_checksum {
            remove_if_present(&temporary_path).await;
            return Err(StorageError::ChecksumMismatch);
        }

        let _guard = self.mutation_lock.lock().await;
        if fs::try_exists(&final_path).await? {
            let existing = file_metadata(&final_path).await?;
            remove_if_present(&temporary_path).await;
            if existing.size_bytes == size_bytes && existing.checksum == checksum {
                return Ok(StoredChunk {
                    outcome: PutChunkOutcome::Existing,
                    relative_path,
                    size_bytes,
                    checksum,
                });
            }
            return Err(StorageError::Conflict);
        }
        if let Err(error) = fs::rename(&temporary_path, &final_path).await {
            remove_if_present(&temporary_path).await;
            return Err(error.into());
        }
        sync_directory(chunk_directory).await?;
        Ok(StoredChunk {
            outcome: PutChunkOutcome::Inserted,
            relative_path,
            size_bytes,
            checksum,
        })
    }

    pub async fn assemble_final_audio(
        &self,
        meeting_id: &str,
        chunks: &[String],
    ) -> Result<FinalAudio, StorageError> {
        validate_meeting_id(meeting_id)?;
        if chunks.is_empty() {
            return Err(StorageError::Conflict);
        }
        for (sequence, relative_path) in chunks.iter().enumerate() {
            if relative_path != &format!("{meeting_id}/chunks/{sequence}.m4a") {
                return Err(StorageError::UnsafePath);
            }
            if !fs::try_exists(self.resolve_relative(relative_path)?).await? {
                return Err(StorageError::Conflict);
            }
        }

        let meeting_directory = self.root.join(meeting_id);
        let final_relative_path = format!("{meeting_id}/recording.m4a");
        let final_path = self.resolve_relative(&final_relative_path)?;
        let _guard = self.mutation_lock.lock().await;
        if fs::try_exists(&final_path).await? {
            let metadata = file_metadata(&final_path).await?;
            return Ok(FinalAudio {
                relative_path: final_relative_path,
                size_bytes: metadata.size_bytes,
                checksum: metadata.checksum,
            });
        }
        let candidate = self.concatenate_chunks(meeting_id, chunks.len()).await?;
        fs::rename(&candidate.output_path, &final_path).await?;
        sync_directory(&meeting_directory).await?;
        let metadata = file_metadata(&final_path).await?;
        Ok(FinalAudio {
            relative_path: final_relative_path,
            size_bytes: metadata.size_bytes,
            checksum: metadata.checksum,
        })
    }

    pub async fn verify_legacy_finalization(
        &self,
        meeting_id: &str,
        last_sequence: i64,
        chunks: &[String],
        expected: &FinalAudioMetadata,
    ) -> Result<Option<VerifiedLegacyFinalization>, StorageError> {
        validate_meeting_id(meeting_id)?;
        let expected_count = usize::try_from(last_sequence)
            .ok()
            .and_then(|sequence| sequence.checked_add(1));
        if expected_count != Some(chunks.len()) || chunks.is_empty() {
            return Ok(None);
        }
        for (sequence, relative_path) in chunks.iter().enumerate() {
            if relative_path != &format!("{meeting_id}/chunks/{sequence}.m4a")
                || !fs::try_exists(self.resolve_relative(relative_path)?).await?
            {
                return Ok(None);
            }
        }
        let final_relative_path = format!("{meeting_id}/recording.m4a");
        if expected.relative_path != final_relative_path || expected.size_bytes < 0 {
            return Ok(None);
        }
        let expected_checksum = match normalize_checksum(&expected.checksum) {
            Ok(checksum) => checksum,
            Err(_) => return Ok(None),
        };
        let expected_size =
            u64::try_from(expected.size_bytes).map_err(|_| StorageError::Conflict)?;
        let final_path = self.resolve_relative(&final_relative_path)?;
        let _guard = self.mutation_lock.lock().await;
        let metadata = file_metadata(&final_path).await?;
        if metadata.size_bytes != expected_size || metadata.checksum != expected_checksum {
            return Ok(None);
        }
        let candidate = self.concatenate_chunks(meeting_id, chunks.len()).await?;
        let candidate_metadata = file_metadata(&candidate.output_path).await?;
        if candidate_metadata != metadata {
            return Ok(None);
        }
        Ok(Some(VerifiedLegacyFinalization {
            meeting_id: meeting_id.to_owned(),
            last_sequence,
            audio: FinalAudioMetadata {
                relative_path: final_relative_path,
                size_bytes: expected.size_bytes,
                checksum: expected_checksum,
            },
        }))
    }

    pub async fn decode_to_pcm16k(&self, relative_path: &str) -> Result<Vec<i16>, StorageError> {
        let path = self.resolve_relative(relative_path)?;
        let output = Command::new(&self.ffmpeg_bin)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&path)
            .args([
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ac",
                "1",
                "-ar",
                "16000",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await?;
        if !output.status.success() {
            return Err(StorageError::Ffmpeg(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        if output.stdout.len() % 2 != 0 {
            return Err(StorageError::Ffmpeg("odd-length PCM output".to_owned()));
        }
        Ok(output
            .stdout
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect())
    }

    pub async fn decode_window_to_pcm16k(
        &self,
        relative_path: &str,
        start_ms: i64,
        duration_ms: i64,
        maximum_samples: usize,
    ) -> Result<Vec<i16>, StorageError> {
        if start_ms < 0 || duration_ms <= 0 || duration_ms > 60_000 || maximum_samples == 0 {
            return Err(StorageError::Conflict);
        }
        let path = self.resolve_relative(relative_path)?;
        let maximum_bytes = maximum_samples
            .checked_mul(2)
            .ok_or(StorageError::TooLarge)?;
        let mut child = Command::new(&self.ffmpeg_bin)
            .args(["-hide_banner", "-loglevel", "error", "-ss"])
            .arg(format!("{:.3}", start_ms as f64 / 1_000.0))
            .arg("-i")
            .arg(&path)
            .args(["-t"])
            .arg(format!("{:.3}", duration_ms as f64 / 1_000.0))
            .args([
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ac",
                "1",
                "-ar",
                "16000",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().ok_or(StorageError::Conflict)?;
        let mut bounded = stdout.take(maximum_bytes as u64 + 1);
        let mut bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
        bounded.read_to_end(&mut bytes).await?;
        if bytes.len() > maximum_bytes {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(StorageError::TooLarge);
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(StorageError::Ffmpeg(
                "bounded window decode failed".to_owned(),
            ));
        }
        if bytes.len() % 2 != 0 {
            return Err(StorageError::Ffmpeg(
                "bounded window decode returned incomplete PCM".to_owned(),
            ));
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect())
    }

    pub async fn open_audio(&self, relative_path: &str) -> Result<File, StorageError> {
        Ok(File::open(self.resolve_relative(relative_path)?).await?)
    }

    pub async fn remove_chunk(&self, relative_path: &str) -> Result<(), StorageError> {
        remove_if_present(&self.resolve_relative(relative_path)?).await;
        Ok(())
    }

    pub async fn delete_meeting(&self, meeting_id: &str) -> Result<(), StorageError> {
        validate_meeting_id(meeting_id)?;
        match fs::remove_dir_all(self.root.join(meeting_id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn concatenate_chunks(
        &self,
        meeting_id: &str,
        chunk_count: usize,
    ) -> Result<ConcatenatedAudio, StorageError> {
        let meeting_directory = self.root.join(meeting_id);
        let operation_id = Uuid::new_v4().hyphenated().to_string();
        let list_name = format!(".concat-{operation_id}.txt");
        let output_name = format!(".recording-{operation_id}.tmp.m4a");
        let list_path = meeting_directory.join(&list_name);
        let output_path = meeting_directory.join(&output_name);
        let temporary_files = TemporaryFiles::new([list_path.clone(), output_path.clone()]);
        let mut list = Vec::new();
        for sequence in 0..chunk_count {
            list.extend_from_slice(format!("file 'chunks/{sequence}.m4a'\n").as_bytes());
        }
        let mut list_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&list_path)
            .await?;
        list_file.write_all(&list).await?;
        list_file.sync_all().await?;
        drop(list_file);
        let output = Command::new(&self.ffmpeg_bin)
            .current_dir(&meeting_directory)
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "concat",
                "-safe",
                "1",
                "-i",
            ])
            .arg(&list_name)
            .args(["-c", "copy"])
            .arg(&output_name)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await?;
        remove_if_present(&list_path).await;
        if !output.status.success() {
            return Err(StorageError::Ffmpeg(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        File::open(&output_path).await?.sync_all().await?;
        Ok(ConcatenatedAudio {
            output_path,
            _temporary_files: temporary_files,
        })
    }

    fn resolve_relative(&self, relative_path: &str) -> Result<PathBuf, StorageError> {
        let path = Path::new(relative_path);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StorageError::UnsafePath);
        }
        Ok(self.root.join(path))
    }
}

struct ConcatenatedAudio {
    output_path: PathBuf,
    _temporary_files: TemporaryFiles<2>,
}

struct TemporaryFiles<const N: usize> {
    paths: [PathBuf; N],
}

impl<const N: usize> TemporaryFiles<N> {
    fn new(paths: [PathBuf; N]) -> Self {
        Self { paths }
    }
}

impl<const N: usize> Drop for TemporaryFiles<N> {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "failed to clean meeting storage temporary file"
                );
            }
        }
    }
}

#[derive(PartialEq, Eq)]
struct FileMetadata {
    size_bytes: u64,
    checksum: String,
}

async fn file_metadata(path: &Path) -> Result<FileMetadata, StorageError> {
    let mut file = File::open(path).await?;
    let size_bytes = file.metadata().await?.len();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(FileMetadata {
        size_bytes,
        checksum: format!("{:x}", hasher.finalize()),
    })
}

fn validate_meeting_id(meeting_id: &str) -> Result<(), StorageError> {
    let parsed = Uuid::parse_str(meeting_id).map_err(|_| StorageError::UnsafePath)?;
    if parsed.hyphenated().to_string() != meeting_id {
        return Err(StorageError::UnsafePath);
    }
    Ok(())
}

fn normalize_checksum(checksum: &str) -> Result<String, StorageError> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageError::InvalidChecksum);
    }
    Ok(checksum.to_ascii_lowercase())
}

async fn remove_if_present(path: &Path) {
    if let Err(error) = fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %error, "failed to clean meeting storage temporary file");
    }
}

async fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path).await?.sync_all().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, path::Path};

    use axum::body::Bytes;
    use futures_util::stream;
    use sha2::{Digest, Sha256};
    use tokio::process::Command;
    use uuid::Uuid;

    use super::{MeetingStorage, PutChunkOutcome, StorageError};
    use crate::meeting::types::FinalAudioMetadata;

    fn checksum(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn byte_stream(
        bytes: Vec<u8>,
    ) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
        stream::iter(vec![Ok(Bytes::from(bytes))])
    }

    fn storage(root: &Path, maximum: u64) -> MeetingStorage {
        MeetingStorage::new(root.to_path_buf(), maximum, "/usr/bin/ffmpeg".into())
    }

    async fn generate_m4a(path: &Path, frequency: u32) -> anyhow::Result<Vec<u8>> {
        let source = format!("sine=frequency={frequency}:duration=0.12");
        let status = Command::new("/usr/bin/ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(source)
            .args(["-c:a", "aac", "-movflags", "+faststart"])
            .arg(path)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "fixture generation failed");
        Ok(tokio::fs::read(path).await?)
    }

    async fn directory_names(path: &Path) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut entries = match tokio::fs::read_dir(path).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    #[tokio::test]
    async fn put_chunk_verifies_sha256_and_removes_failed_temp_file() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 1024);
        let error = storage
            .put_chunk(
                &meeting_id,
                0,
                &"0".repeat(64),
                byte_stream(b"actual bytes".to_vec()),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::ChecksumMismatch));
        assert!(
            directory_names(&directory.path().join(&meeting_id).join("chunks"))
                .await?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn put_chunk_enforces_maximum_size_and_removes_failed_temp_file() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 4);
        let bytes = b"five!".to_vec();
        let error = storage
            .put_chunk(&meeting_id, 0, &checksum(&bytes), byte_stream(bytes))
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::TooLarge));
        assert!(
            directory_names(&directory.path().join(&meeting_id).join("chunks"))
                .await?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn put_chunk_atomically_renames_to_immutable_sequence_path() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 1024);
        let bytes = b"complete chunk".to_vec();
        let stored = storage
            .put_chunk(
                &meeting_id,
                7,
                &checksum(&bytes),
                byte_stream(bytes.clone()),
            )
            .await?;

        assert_eq!(stored.outcome, PutChunkOutcome::Inserted);
        assert_eq!(stored.relative_path, format!("{meeting_id}/chunks/7.m4a"));
        assert_eq!(
            tokio::fs::read(directory.path().join(&stored.relative_path)).await?,
            bytes
        );
        assert_eq!(
            directory_names(&directory.path().join(&meeting_id).join("chunks")).await?,
            vec!["7.m4a"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn identical_retry_is_accepted_but_different_content_conflicts() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 1024);
        let original = b"immutable".to_vec();
        let original_checksum = checksum(&original);
        storage
            .put_chunk(
                &meeting_id,
                0,
                &original_checksum,
                byte_stream(original.clone()),
            )
            .await?;

        let retry = storage
            .put_chunk(
                &meeting_id,
                0,
                &original_checksum,
                byte_stream(original.clone()),
            )
            .await?;
        assert_eq!(retry.outcome, PutChunkOutcome::Existing);

        let changed = b"different".to_vec();
        let error = storage
            .put_chunk(&meeting_id, 0, &checksum(&changed), byte_stream(changed))
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::Conflict));
        assert_eq!(
            tokio::fs::read(directory.path().join(&retry.relative_path)).await?,
            original
        );
        Ok(())
    }

    #[tokio::test]
    async fn storage_rejects_unsafe_meeting_and_audio_paths() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let storage = storage(directory.path(), 1024);
        let bytes = b"escape".to_vec();

        let upload_error = storage
            .put_chunk("../escape", 0, &checksum(&bytes), byte_stream(bytes))
            .await
            .unwrap_err();
        assert!(matches!(upload_error, StorageError::UnsafePath));
        let decode_error = storage
            .decode_to_pcm16k("../outside.m4a")
            .await
            .unwrap_err();
        assert!(matches!(decode_error, StorageError::UnsafePath));
        assert!(!directory.path().join("escape").exists());
        Ok(())
    }

    #[tokio::test]
    async fn generated_m4a_chunks_assemble_and_decode_to_pcm16k() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let fixture_directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 1024 * 1024);
        let first = generate_m4a(&fixture_directory.path().join("first.m4a"), 440).await?;
        let second = generate_m4a(&fixture_directory.path().join("second.m4a"), 660).await?;
        let first = storage
            .put_chunk(&meeting_id, 0, &checksum(&first), byte_stream(first))
            .await?;
        let second = storage
            .put_chunk(&meeting_id, 1, &checksum(&second), byte_stream(second))
            .await?;

        let audio = storage
            .assemble_final_audio(
                &meeting_id,
                &[first.relative_path.clone(), second.relative_path.clone()],
            )
            .await?;
        let samples = storage.decode_to_pcm16k(&audio.relative_path).await?;

        assert_eq!(audio.relative_path, format!("{meeting_id}/recording.m4a"));
        assert!(audio.size_bytes > 0);
        let final_bytes = tokio::fs::read(directory.path().join(&audio.relative_path)).await?;
        assert_eq!(audio.size_bytes, final_bytes.len() as u64);
        assert_eq!(audio.checksum, checksum(&final_bytes));
        assert!(
            samples.len() > 2_000,
            "assembled audio should contain both fixtures"
        );
        storage.delete_meeting(&meeting_id).await?;
        assert!(!directory.path().join(meeting_id).exists());
        Ok(())
    }

    #[tokio::test]
    async fn window_decode_caps_memory_independently_of_source_duration() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let relative_path = format!("{meeting_id}/recording.m4a");
        let absolute_path = directory.path().join(&relative_path);
        tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
        generate_m4a(&absolute_path, 440).await?;
        let storage = storage(directory.path(), 1024 * 1024);

        let samples = storage
            .decode_window_to_pcm16k(&relative_path, 0, 1_000, 16_000)
            .await?;

        assert!(samples.len() <= 16_000);
        Ok(())
    }

    #[tokio::test]
    async fn window_decode_kills_an_unbounded_decoder_after_one_window() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let relative_path = format!("{meeting_id}/recording.m4a");
        let absolute_path = directory.path().join(&relative_path);
        tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
        tokio::fs::write(&absolute_path, b"fixture").await?;
        let ffmpeg = directory.path().join("fake-ffmpeg");
        tokio::fs::write(
            &ffmpeg,
            "#!/bin/sh\nwhile true; do printf '0000000000000000'; done\n",
        )
        .await?;
        let mut permissions = std::fs::metadata(&ffmpeg)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&ffmpeg, permissions)?;
        let storage = MeetingStorage::new(directory.path().into(), 1024, ffmpeg);

        let error = storage
            .decode_window_to_pcm16k(&relative_path, 0, 1_000, 16_000)
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::TooLarge));
        Ok(())
    }

    #[tokio::test]
    async fn assemble_failure_removes_all_temporary_files() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let ffmpeg = directory.path().join("fake-ffmpeg");
        tokio::fs::write(
            &ffmpeg,
            concat!(
                "#!/bin/sh\n",
                "output_name=\n",
                "for argument in \"$@\"; do output_name=$argument; done\n",
                "mkdir recording.m4a\n",
                "printf fake > \"$output_name\"\n",
            ),
        )
        .await?;
        let mut permissions = std::fs::metadata(&ffmpeg)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&ffmpeg, permissions)?;
        let storage = MeetingStorage::new(directory.path().into(), 1024, ffmpeg);
        let bytes = b"chunk".to_vec();
        let chunk = storage
            .put_chunk(&meeting_id, 0, &checksum(&bytes), byte_stream(bytes))
            .await?;

        let error = storage
            .assemble_final_audio(&meeting_id, &[chunk.relative_path])
            .await
            .unwrap_err();

        assert!(matches!(error, StorageError::Io(_)));
        let names = directory_names(&directory.path().join(&meeting_id)).await?;
        assert!(
            names
                .iter()
                .all(|name| !name.starts_with(".concat-") && !name.starts_with(".recording-")),
            "temporary files remained after failure: {names:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_finalization_proof_matches_only_the_exact_chunk_prefix() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let fixture_directory = tempfile::tempdir()?;
        let meeting_id = Uuid::new_v4().to_string();
        let storage = storage(directory.path(), 1024 * 1024);
        let first = generate_m4a(&fixture_directory.path().join("legacy-first.m4a"), 440).await?;
        let second = generate_m4a(&fixture_directory.path().join("legacy-second.m4a"), 660).await?;
        let first = storage
            .put_chunk(&meeting_id, 0, &checksum(&first), byte_stream(first))
            .await?;
        let second = storage
            .put_chunk(&meeting_id, 1, &checksum(&second), byte_stream(second))
            .await?;
        let final_audio = storage
            .assemble_final_audio(&meeting_id, &[first.relative_path.clone()])
            .await?;
        let expected = FinalAudioMetadata {
            relative_path: final_audio.relative_path,
            size_bytes: i64::try_from(final_audio.size_bytes)?,
            checksum: final_audio.checksum,
        };

        let exact = storage
            .verify_legacy_finalization(
                &meeting_id,
                0,
                std::slice::from_ref(&first.relative_path),
                &expected,
            )
            .await?;
        let includes_later_chunk = storage
            .verify_legacy_finalization(
                &meeting_id,
                1,
                &[first.relative_path, second.relative_path],
                &expected,
            )
            .await?;

        assert!(exact.is_some());
        assert!(includes_later_chunk.is_none());
        assert_eq!(
            directory_names(&directory.path().join(&meeting_id)).await?,
            vec!["chunks", "recording.m4a"]
        );
        Ok(())
    }
}
