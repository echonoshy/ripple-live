use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::adapters::ModelAdapters;

use super::types::TranscriptSegment;
use super::{storage::MeetingStorage, store::MeetingStore};

const PCM_SAMPLE_RATE: usize = 16_000;
const DEFAULT_WINDOW_MS: usize = 60_000;
const DEFAULT_OVERLAP_MS: usize = 1_000;
const MAX_ASR_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct MeetingProcessor {
    store: MeetingStore,
    storage: MeetingStorage,
    adapters: ModelAdapters,
    asr_permits: Arc<Semaphore>,
    window_samples: usize,
    overlap_samples: usize,
    scheduled_chunks: Arc<Mutex<HashSet<(String, i64)>>>,
}

impl MeetingProcessor {
    pub fn new(
        store: MeetingStore,
        storage: MeetingStorage,
        adapters: ModelAdapters,
        maximum_concurrency: usize,
    ) -> anyhow::Result<Self> {
        if maximum_concurrency == 0 {
            bail!("meeting ASR concurrency must be positive");
        }
        Ok(Self {
            store,
            storage,
            adapters,
            asr_permits: Arc::new(Semaphore::new(maximum_concurrency)),
            window_samples: DEFAULT_WINDOW_MS * PCM_SAMPLE_RATE / 1_000,
            overlap_samples: DEFAULT_OVERLAP_MS * PCM_SAMPLE_RATE / 1_000,
            scheduled_chunks: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn spawn_transcribe_chunk(
        &self,
        meeting_id: String,
        sequence: i64,
        start_ms: i64,
        end_ms: i64,
        relative_path: String,
    ) -> bool {
        let key = (meeting_id.clone(), sequence);
        if !self
            .scheduled_chunks
            .lock()
            .expect("meeting ASR schedule lock poisoned")
            .insert(key)
        {
            return false;
        }
        let processor = self.clone();
        tokio::spawn(async move {
            if processor
                .transcribe_chunk(&meeting_id, sequence, start_ms, end_ms, &relative_path)
                .await
                .is_err()
            {
                tracing::warn!(
                    meeting_id,
                    sequence,
                    "meeting provisional transcription failed"
                );
            }
        });
        true
    }

    pub fn spawn_finalize_transcript(
        &self,
        meeting_id: String,
        relative_path: String,
    ) -> JoinHandle<()> {
        let processor = self.clone();
        tokio::spawn(async move {
            if processor
                .finalize_transcript(&meeting_id, &relative_path)
                .await
                .is_err()
            {
                tracing::warn!(meeting_id, "meeting final transcription failed");
            }
        })
    }

    pub async fn transcribe_chunk(
        &self,
        meeting_id: &str,
        sequence: i64,
        start_ms: i64,
        end_ms: i64,
        relative_path: &str,
    ) -> anyhow::Result<Option<TranscriptSegment>> {
        if sequence < 0 || start_ms < 0 || end_ms <= start_ms {
            bail!("invalid provisional transcript range");
        }
        let _permit = self
            .asr_permits
            .clone()
            .acquire_owned()
            .await
            .context("meeting ASR semaphore closed")?;
        let pcm = self
            .storage
            .decode_to_pcm16k(relative_path)
            .await
            .context("decode provisional meeting audio")?;
        let text = self.transcribe_with_retry(&pcm).await?;
        let segment = (!text.trim().is_empty()).then(|| TranscriptSegment {
            id: sequence,
            start_ms,
            end_ms,
            text: text.trim().to_owned(),
            provisional: true,
        });
        self.store
            .replace_provisional_segment(meeting_id, sequence, segment.as_ref())
            .await
            .context("persist provisional meeting transcript")?;
        Ok(segment)
    }

    pub async fn finalize_transcript(
        &self,
        meeting_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<Vec<TranscriptSegment>> {
        let pcm = self
            .storage
            .decode_to_pcm16k(relative_path)
            .await
            .context("decode final meeting audio")?;
        let recorded_duration_ms = self
            .store
            .recorded_duration_ms(meeting_id)
            .await
            .context("load authoritative meeting duration")?;
        let timeline_end_ms = if recorded_duration_ms > 0 {
            recorded_duration_ms
        } else {
            samples_to_ms(pcm.len())
        };
        let ranges = window_ranges(pcm.len(), self.window_samples, self.overlap_samples);
        let mut raw = Vec::with_capacity(ranges.len());
        for (start_sample, end_sample) in ranges {
            let _permit = self
                .asr_permits
                .clone()
                .acquire_owned()
                .await
                .context("meeting ASR semaphore closed")?;
            let text = self
                .transcribe_with_retry(&pcm[start_sample..end_sample])
                .await?;
            raw.push((
                samples_to_ms(start_sample).min(timeline_end_ms),
                samples_to_ms(end_sample).min(timeline_end_ms),
                text,
            ));
        }
        let borrowed = raw
            .iter()
            .map(|(start_ms, end_ms, text)| (*start_ms, *end_ms, text.as_str()))
            .collect::<Vec<_>>();
        let segments = reconcile_overlap(&borrowed);
        self.store
            .replace_transcript(meeting_id, &segments)
            .await
            .context("replace final meeting transcript")?;
        Ok(segments)
    }

    async fn transcribe_with_retry(&self, pcm: &[i16]) -> anyhow::Result<String> {
        let samples = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();
        let mut last_error = None;
        for attempt in 0..MAX_ASR_ATTEMPTS {
            match self.adapters.transcribe(&samples).await {
                Ok(text) => return Ok(text),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < MAX_ASR_ATTEMPTS {
                        tokio::task::yield_now().await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("meeting ASR failed")))
            .context("meeting ASR retries exhausted")
    }

    #[cfg(test)]
    fn with_window_for_test(mut self, window_ms: usize, overlap_ms: usize) -> Self {
        assert!(window_ms > overlap_ms);
        self.window_samples = window_ms * PCM_SAMPLE_RATE / 1_000;
        self.overlap_samples = overlap_ms * PCM_SAMPLE_RATE / 1_000;
        self
    }
}

fn samples_to_ms(samples: usize) -> i64 {
    (samples as i64) * 1_000 / PCM_SAMPLE_RATE as i64
}

fn window_ranges(total: usize, window: usize, overlap: usize) -> Vec<(usize, usize)> {
    if total == 0 || window == 0 || overlap >= window {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut start: usize = 0;
    loop {
        let end = start.saturating_add(window).min(total);
        ranges.push((start, end));
        if end == total {
            break;
        }
        start = end - overlap;
    }
    ranges
}

pub fn reconcile_overlap(segments: &[(i64, i64, &str)]) -> Vec<TranscriptSegment> {
    let mut merged: Vec<TranscriptSegment> = Vec::new();
    for &(start_ms, end_ms, text) in segments {
        let text = text.trim();
        if text.is_empty() || start_ms < 0 || end_ms <= start_ms {
            continue;
        }
        let mut adjusted_text = text;
        let mut adjusted_start = start_ms;
        if let Some(previous) = merged.last() {
            let duplicate_chars = longest_boundary_overlap(&previous.text, adjusted_text);
            if duplicate_chars > 0 {
                adjusted_text = &adjusted_text[char_byte_offset(adjusted_text, duplicate_chars)..];
            }
            adjusted_start = adjusted_start.max(previous.end_ms);
        }
        let adjusted_text = adjusted_text.trim();
        if adjusted_text.is_empty() || end_ms <= adjusted_start {
            continue;
        }
        merged.push(TranscriptSegment {
            id: merged.len() as i64,
            start_ms: adjusted_start,
            end_ms,
            text: adjusted_text.to_owned(),
            provisional: false,
        });
    }
    merged
}

fn longest_boundary_overlap(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let maximum = left.len().min(right.len());
    (1..=maximum)
        .rev()
        .find(|&length| left[left.len() - length..] == right[..length])
        .unwrap_or(0)
}

fn char_byte_offset(value: &str, chars: usize) -> usize {
    value
        .char_indices()
        .nth(chars)
        .map(|(offset, _)| offset)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use super::{MeetingProcessor, reconcile_overlap, window_ranges};
    use crate::{
        adapters::ModelAdapters,
        config::Settings,
        context::ContextStore,
        meeting::{storage::MeetingStorage, store::MeetingStore, types::TranscriptSegment},
    };
    use tokio::process::Command;

    async fn fixture(
        results: Vec<Result<&str, &str>>,
        delay: Duration,
        maximum_concurrency: usize,
    ) -> anyhow::Result<(
        tempfile::TempDir,
        MeetingStore,
        MeetingStorage,
        MeetingProcessor,
        crate::adapters::TranscriptionTestProbe,
        String,
        String,
    )> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store.create_with_status("user-a", "task-4", 1.0).await?;
        store
            .record_chunk(&meeting.id, 0, 0, 360, "fixture", 1)
            .await?;
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            "/usr/bin/ffmpeg".into(),
        );
        let relative_path = format!("{}/chunks/0.m4a", meeting.id);
        let absolute_path = directory.path().join("audio").join(&relative_path);
        tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
        generate_m4a(&absolute_path, 0.36).await?;

        let settings = Settings::from_env()?;
        let (adapters, probe) = ModelAdapters::with_transcription_test_results(
            settings,
            results
                .into_iter()
                .map(|result| result.map(str::to_owned).map_err(str::to_owned))
                .collect(),
            delay,
        )?;
        let processor = MeetingProcessor::new(
            store.clone(),
            storage.clone(),
            adapters,
            maximum_concurrency,
        )?;
        Ok((
            directory,
            store,
            storage,
            processor,
            probe,
            meeting.id,
            relative_path,
        ))
    }

    async fn generate_m4a(path: &Path, seconds: f32) -> anyhow::Result<()> {
        let status = Command::new("/usr/bin/ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=440:duration={seconds}"),
                "-c:a",
                "aac",
            ])
            .arg(path)
            .status()
            .await?;
        anyhow::ensure!(status.success(), "fixture generation failed");
        Ok(())
    }

    #[test]
    fn removes_duplicate_chinese_boundary_text_and_overlap_time() {
        let merged = reconcile_overlap(&[
            (0, 15_000, "今天讨论发布计划"),
            (14_000, 30_000, "发布计划下周开始"),
        ]);

        assert_eq!(merged[0].text, "今天讨论发布计划");
        assert_eq!(merged[1].text, "下周开始");
        assert!(
            merged
                .windows(2)
                .all(|pair| pair[0].end_ms <= pair[1].start_ms)
        );
    }

    #[test]
    fn drops_silence_and_preserves_stable_millisecond_offsets() {
        let merged = reconcile_overlap(&[
            (0, 1_000, "  "),
            (1_000, 2_001, "第一段"),
            (2_001, 3_003, "第二段"),
        ]);

        assert_eq!(merged.len(), 2);
        assert_eq!((merged[0].start_ms, merged[0].end_ms), (1_000, 2_001));
        assert_eq!((merged[1].start_ms, merged[1].end_ms), (2_001, 3_003));
        assert!(
            merged
                .windows(2)
                .all(|pair| pair[0].end_ms <= pair[1].start_ms)
        );
    }

    #[test]
    fn final_windows_use_stable_sample_derived_offsets() {
        assert_eq!(
            window_ranges(4_800, 3_200, 1_600),
            vec![(0, 3_200), (1_600, 4_800)]
        );
    }

    #[tokio::test]
    async fn retries_asr_and_replaces_stale_provisional_segment() -> anyhow::Result<()> {
        let (_directory, store, _storage, processor, probe, meeting_id, relative_path) = fixture(
            vec![
                Err("temporary-1"),
                Err("temporary-2"),
                Ok("旧文本"),
                Ok("新文本"),
            ],
            Duration::ZERO,
            1,
        )
        .await?;

        processor
            .transcribe_chunk(&meeting_id, 0, 0, 360, &relative_path)
            .await?;
        processor
            .transcribe_chunk(&meeting_id, 0, 0, 360, &relative_path)
            .await?;

        let meeting = store.get_owned("user-a", &meeting_id).await?.unwrap();
        assert_eq!(probe.attempts(), 4);
        assert_eq!(
            meeting.transcript,
            vec![TranscriptSegment {
                id: 0,
                start_ms: 0,
                end_ms: 360,
                text: "新文本".to_owned(),
                provisional: true,
            }]
        );
        Ok(())
    }

    #[tokio::test]
    async fn finalization_replaces_provisional_rows_with_monotonic_final_rows() -> anyhow::Result<()>
    {
        let (_directory, store, _storage, processor, _probe, meeting_id, relative_path) = fixture(
            vec![Ok("今天讨论发布计划"), Ok("发布计划下周开始")],
            Duration::ZERO,
            1,
        )
        .await?;
        store
            .replace_transcript(
                &meeting_id,
                &[TranscriptSegment {
                    id: 99,
                    start_ms: 0,
                    end_ms: 10,
                    text: "过期草稿".to_owned(),
                    provisional: true,
                }],
            )
            .await?;
        let processor = processor.with_window_for_test(250, 100);

        let segments = processor
            .finalize_transcript(&meeting_id, &relative_path)
            .await?;

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "今天讨论发布计划");
        assert_eq!(segments[1].text, "下周开始");
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 250));
        assert_eq!(segments[1].start_ms, 250);
        assert_eq!(segments[1].end_ms, 360);
        assert!(segments.iter().all(|segment| !segment.provisional));
        assert!(
            segments
                .windows(2)
                .all(|pair| pair[0].end_ms <= pair[1].start_ms)
        );
        let persisted = store.get_owned("user-a", &meeting_id).await?.unwrap();
        assert_eq!(persisted.transcript, segments);
        Ok(())
    }

    #[tokio::test]
    async fn semaphore_bounds_catch_up_concurrency() -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, probe, meeting_id, relative_path) = fixture(
            vec![Ok("一"), Ok("二"), Ok("三"), Ok("四")],
            Duration::from_millis(40),
            2,
        )
        .await?;
        let mut tasks = Vec::new();
        for sequence in 0..4 {
            let processor = processor.clone();
            let meeting_id = meeting_id.clone();
            let relative_path = relative_path.clone();
            tasks.push(tokio::spawn(async move {
                processor
                    .transcribe_chunk(
                        &meeting_id,
                        sequence,
                        sequence * 400,
                        sequence * 400 + 360,
                        &relative_path,
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await??;
        }

        assert_eq!(probe.maximum_active(), 2);
        assert_eq!(probe.active(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn failed_asr_releases_the_only_permit() -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, probe, meeting_id, relative_path) = fixture(
            vec![Err("one"), Err("two"), Err("three"), Ok("恢复")],
            Duration::from_millis(5),
            1,
        )
        .await?;
        assert!(
            processor
                .transcribe_chunk(&meeting_id, 0, 0, 360, &relative_path)
                .await
                .is_err()
        );
        processor
            .transcribe_chunk(&meeting_id, 1, 360, 720, &relative_path)
            .await?;

        assert_eq!(probe.attempts(), 4);
        assert_eq!(probe.maximum_active(), 1);
        assert_eq!(probe.active(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_provisional_schedule_runs_asr_only_once() -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, probe, meeting_id, relative_path) =
            fixture(vec![Ok("只应执行一次"), Ok("重复调度")], Duration::ZERO, 1).await?;

        assert!(processor.spawn_transcribe_chunk(
            meeting_id.clone(),
            0,
            0,
            360,
            relative_path.clone(),
        ));
        assert!(!processor.spawn_transcribe_chunk(meeting_id, 0, 0, 360, relative_path));
        for _ in 0..50 {
            if probe.attempts() == 1 && probe.active() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(probe.attempts(), 1);
        Ok(())
    }
}
