use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, bail};
use serde::Deserialize;
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::adapters::ModelAdapters;

use super::types::{
    MAX_MEETING_CHUNK_SEQUENCE, MeetingArtifact, MeetingTodoDraft, StoredChunkMetadata,
    TranscriptJobClaim, TranscriptSegment,
};
use super::{storage::MeetingStorage, store::MeetingStore};

const PCM_SAMPLE_RATE: usize = 16_000;
const DEFAULT_WINDOW_MS: usize = 58_000;
const DEFAULT_OVERLAP_MS: usize = 1_000;
const MAX_ASR_ATTEMPTS: usize = 3;
const MAX_ORGANIZATION_ATTEMPTS: usize = 3;
const VAD_FRAME_MS: i64 = 20;
const VAD_ENERGY_THRESHOLD: i64 = 500;
const VAD_MIN_SPEECH_MS: i64 = 200;
const VAD_MAX_SILENCE_MS: i64 = 500;
const VAD_MAX_SPAN_MS: i64 = 20_000;
const VAD_SPAN_OVERLAP_MS: i64 = 1_000;
const FALLBACK_NON_SILENCE_ENERGY: i64 = 16;
const FALLBACK_SPAN_MS: i64 = 10_000;
const MAX_DECODE_PADDING_MS: usize = 100;
const ORGANIZATION_SECTION_CHARS: usize = 12_000;
const MAX_ORGANIZATION_SUMMARY_LAYERS: usize = 8;

#[derive(Clone)]
pub struct MeetingProcessor {
    store: MeetingStore,
    storage: MeetingStorage,
    adapters: ModelAdapters,
    asr_permits: Arc<Semaphore>,
    window_samples: usize,
    overlap_samples: usize,
    scheduled_chunks: Arc<Mutex<HashSet<(String, i64)>>>,
    retry_delays: Arc<Vec<Duration>>,
    organization_input_chars: usize,
}

struct ChunkFlightGuard {
    scheduled: Arc<Mutex<HashSet<(String, i64)>>>,
    key: Option<(String, i64)>,
}

impl Drop for ChunkFlightGuard {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.scheduled
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key);
        }
    }
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
            retry_delays: Arc::new(vec![Duration::from_millis(50), Duration::from_millis(100)]),
            organization_input_chars: ORGANIZATION_SECTION_CHARS,
        })
    }

    pub fn spawn_transcribe_chunk(
        &self,
        meeting_id: String,
        sequence: i64,
        start_ms: i64,
        end_ms: i64,
        relative_path: String,
    ) -> Option<JoinHandle<()>> {
        let key = (meeting_id.clone(), sequence);
        {
            let mut scheduled = self
                .scheduled_chunks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !scheduled.insert(key.clone()) {
                return None;
            }
        }
        let guard = ChunkFlightGuard {
            scheduled: Arc::clone(&self.scheduled_chunks),
            key: Some(key),
        };
        let processor = self.clone();
        Some(tokio::spawn(async move {
            let _guard = guard;
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
        }))
    }

    pub fn spawn_finalize_transcript(
        &self,
        meeting_id: String,
        relative_path: String,
    ) -> JoinHandle<()> {
        let processor = self.clone();
        tokio::spawn(async move {
            match processor
                .finalize_transcript(&meeting_id, &relative_path)
                .await
            {
                Ok(Some(_)) => {
                    if let Err(error) = processor.organize_meeting(&meeting_id).await {
                        tracing::warn!(meeting_id, error = %format!("{error:#}"), "meeting organization failed");
                    }
                }
                Ok(None) => {
                    if let Err(error) = processor.organize_meeting(&meeting_id).await {
                        tracing::warn!(meeting_id, error = %format!("{error:#}"), "meeting organization failed");
                    }
                }
                Err(_) => {
                    tracing::warn!(meeting_id, "meeting final transcription failed");
                }
            }
        })
    }

    pub fn spawn_organize_meeting(&self, meeting_id: String) -> JoinHandle<()> {
        let processor = self.clone();
        tokio::spawn(async move {
            if let Err(error) = processor.organize_meeting(&meeting_id).await {
                tracing::warn!(meeting_id, error = %format!("{error:#}"), "meeting organization failed");
            }
        })
    }

    pub async fn organize_meeting(
        &self,
        meeting_id: &str,
    ) -> anyhow::Result<Option<MeetingArtifact>> {
        let claim = self.store.claim_organization_job(meeting_id).await?;
        let TranscriptJobClaim::Claimed { attempt } = claim else {
            return Ok(None);
        };
        match self.process_meeting_organization(meeting_id, attempt).await {
            Ok(artifact) => {
                if let Err(error) = self
                    .store
                    .complete_organization_job(meeting_id, attempt, &artifact)
                    .await
                {
                    self.store
                        .fail_organization_job(meeting_id, attempt)
                        .await?;
                    return Err(error).context("complete meeting organization job");
                }
                Ok(Some(artifact))
            }
            Err(error) => {
                self.store
                    .fail_organization_job(meeting_id, attempt)
                    .await
                    .context("record meeting organization failure")?;
                Err(error)
            }
        }
    }

    async fn process_meeting_organization(
        &self,
        meeting_id: &str,
        attempt: i64,
    ) -> anyhow::Result<MeetingArtifact> {
        let transcript = self
            .store
            .final_transcript(meeting_id)
            .await
            .context("load final meeting transcript")?;
        if transcript.is_empty() {
            return Ok(MeetingArtifact {
                title: "未检测到语音内容".to_owned(),
                summary: "录音中未检测到可转写的语音内容。".to_owned(),
                todos: Vec::new(),
            });
        }
        let sections = meeting_organization_sections(&transcript, self.organization_input_chars)?;
        let organized_input = if sections.len() == 1 {
            sections[0].clone()
        } else {
            let mut summaries = Vec::with_capacity(sections.len());
            for (index, section) in sections.into_iter().enumerate() {
                if !self
                    .store
                    .heartbeat_organization_job(meeting_id, attempt)
                    .await?
                {
                    bail!("organization job lease was lost");
                }
                let summary = self.adapters.summarize_meeting_section(&section).await?;
                if summary.chars().count() > self.organization_input_chars {
                    bail!("meeting section summary exceeded the organization input limit");
                }
                summaries.push(format!("分段摘要 {}:\n{}", index + 1, summary));
            }
            self.compact_organization_summaries(meeting_id, attempt, summaries)
                .await?
        };
        if organized_input.chars().count() > self.organization_input_chars {
            bail!("final meeting organization input exceeded the hard limit");
        }
        if !self
            .store
            .heartbeat_organization_job(meeting_id, attempt)
            .await?
        {
            bail!("organization job lease was lost");
        }
        let timeline_start = transcript
            .iter()
            .map(|segment| segment.start_ms)
            .min()
            .context("final meeting transcript was empty")?;
        let timeline_end = transcript
            .iter()
            .map(|segment| segment.end_ms)
            .max()
            .context("final meeting transcript was empty")?;
        let mut last_error = None;
        for provider_attempt in 0..MAX_ORGANIZATION_ATTEMPTS {
            if !self
                .store
                .heartbeat_organization_job(meeting_id, attempt)
                .await?
            {
                bail!("organization job lease was lost");
            }
            match self
                .adapters
                .organize_meeting_artifact(&organized_input)
                .await
                .and_then(|output| validated_meeting_artifact(output, timeline_start, timeline_end))
            {
                Ok(artifact) => return Ok(artifact),
                Err(error) => {
                    last_error = Some(error);
                    if let Some(delay) = self.retry_delays.get(provider_attempt) {
                        tokio::time::sleep(*delay).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("meeting organization failed")))
            .context("meeting organization retries exhausted")
    }

    async fn compact_organization_summaries(
        &self,
        meeting_id: &str,
        attempt: i64,
        mut summaries: Vec<String>,
    ) -> anyhow::Result<String> {
        for layer in 0..MAX_ORGANIZATION_SUMMARY_LAYERS {
            let joined = summaries.join("\n\n");
            let previous_chars = joined.chars().count();
            if previous_chars <= self.organization_input_chars {
                return Ok(joined);
            }
            let batches = organization_summary_batches(&summaries, self.organization_input_chars)?;
            let mut compacted = Vec::with_capacity(batches.len());
            for (batch_index, batch) in batches.into_iter().enumerate() {
                let batch_input = batch.join("\n\n");
                if batch_input.chars().count() > self.organization_input_chars {
                    bail!("meeting summary batch exceeded the organization input limit");
                }
                if !self
                    .store
                    .heartbeat_organization_job(meeting_id, attempt)
                    .await?
                {
                    bail!("organization job lease was lost");
                }
                let summary = self
                    .adapters
                    .summarize_meeting_section(&batch_input)
                    .await?;
                let compacted_item =
                    format!("层级摘要 {}-{}:\n{}", layer + 1, batch_index + 1, summary);
                if compacted_item.chars().count() >= batch_input.chars().count()
                    || compacted_item.chars().count() > self.organization_input_chars
                {
                    bail!("meeting summary compaction did not shrink safely");
                }
                compacted.push(compacted_item);
            }
            let compacted_chars = compacted.join("\n\n").chars().count();
            if compacted_chars >= previous_chars {
                bail!("meeting summary compaction made no progress");
            }
            summaries = compacted;
        }
        let joined = summaries.join("\n\n");
        if joined.chars().count() <= self.organization_input_chars {
            Ok(joined)
        } else {
            bail!("meeting summary compaction exceeded the maximum layer count")
        }
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
        let duration_ms = end_ms - start_ms;
        if duration_ms > DEFAULT_WINDOW_MS as i64 {
            bail!("provisional meeting chunk exceeds decode window");
        }
        let _permit = self
            .asr_permits
            .clone()
            .acquire_owned()
            .await
            .context("meeting ASR semaphore closed")?;
        let pcm = self
            .storage
            .decode_window_to_pcm16k(
                relative_path,
                0,
                duration_ms,
                duration_ms as usize * PCM_SAMPLE_RATE / 1_000,
            )
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
    ) -> anyhow::Result<Option<Vec<TranscriptSegment>>> {
        let claim = self.store.claim_transcript_job(meeting_id).await?;
        let claim = if claim == TranscriptJobClaim::Missing {
            self.store.enqueue_transcript_job(meeting_id).await?;
            self.store.claim_transcript_job(meeting_id).await?
        } else {
            claim
        };
        let TranscriptJobClaim::Claimed { attempt } = claim else {
            return Ok(None);
        };
        match self
            .process_final_transcript(meeting_id, attempt, relative_path)
            .await
        {
            Ok(segments) => {
                if let Err(error) = self
                    .store
                    .complete_transcript_job(meeting_id, attempt, &segments)
                    .await
                {
                    self.store.fail_transcript_job(meeting_id, attempt).await?;
                    return Err(error).context("complete transcript processing job");
                }
                Ok(Some(segments))
            }
            Err(error) => {
                self.store
                    .fail_transcript_job(meeting_id, attempt)
                    .await
                    .context("record transcript processing failure")?;
                Err(error)
            }
        }
    }

    async fn process_final_transcript(
        &self,
        meeting_id: &str,
        attempt: i64,
        relative_path: &str,
    ) -> anyhow::Result<Vec<TranscriptSegment>> {
        let chunks = self
            .store
            .verified_chunks(meeting_id, MAX_MEETING_CHUNK_SEQUENCE)
            .await
            .context("load verified meeting chunks")?;
        if chunks.is_empty() {
            bail!("meeting has no verified chunks for transcription");
        }
        validate_chunk_timeline(&chunks)?;
        let window_ms = samples_to_ms(self.window_samples).max(1);
        let overlap_ms = samples_to_ms(self.overlap_samples);
        let groups = continuity_groups(&chunks);
        let windows = continuity_windows(&groups, window_ms, overlap_ms);
        let mut raw = Vec::new();
        for window in windows {
            if !self
                .store
                .heartbeat_transcript_job(meeting_id, attempt)
                .await?
            {
                bail!("transcript job lease was lost");
            }
            let _permit = self
                .asr_permits
                .clone()
                .acquire_owned()
                .await
                .context("meeting ASR semaphore closed")?;
            let duration_ms = window.end_ms - window.start_ms;
            let requested_samples = duration_ms as usize * PCM_SAMPLE_RATE / 1_000;
            let maximum_samples =
                requested_samples + MAX_DECODE_PADDING_MS * PCM_SAMPLE_RATE / 1_000;
            let mut pcm = self
                .storage
                .decode_window_to_pcm16k(
                    relative_path,
                    window.media_start_ms,
                    duration_ms,
                    maximum_samples,
                )
                .await
                .context("decode bounded meeting audio window")?;
            pcm.truncate(requested_samples);
            for span in transcription_spans(&pcm) {
                if !self
                    .store
                    .heartbeat_transcript_job(meeting_id, attempt)
                    .await?
                {
                    bail!("transcript job lease was lost");
                }
                let start_sample = span.start_ms as usize * PCM_SAMPLE_RATE / 1_000;
                let end_sample = (span.end_ms as usize * PCM_SAMPLE_RATE / 1_000).min(pcm.len());
                if end_sample <= start_sample {
                    continue;
                }
                let text = self
                    .transcribe_with_retry(&pcm[start_sample..end_sample])
                    .await?;
                raw.push((
                    window.start_ms + span.start_ms,
                    (window.start_ms + span.end_ms).min(window.end_ms),
                    text,
                ));
            }
        }
        let borrowed = raw
            .iter()
            .map(|(start_ms, end_ms, text)| (*start_ms, *end_ms, text.as_str()))
            .collect::<Vec<_>>();
        Ok(reconcile_overlap(&borrowed))
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
                    if let Some(delay) = self.retry_delays.get(attempt) {
                        tokio::time::sleep(*delay).await;
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

    #[cfg(test)]
    fn with_retry_delays_for_test(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = Arc::new(delays);
        self
    }

    #[cfg(test)]
    fn with_organization_input_chars_for_test(mut self, maximum_chars: usize) -> Self {
        self.organization_input_chars = maximum_chars;
        self
    }
}

fn validated_meeting_artifact(
    output: crate::adapters::ResponsesOutput,
    timeline_start: i64,
    timeline_end: i64,
) -> anyhow::Result<MeetingArtifact> {
    if output.function_calls.is_empty() {
        bail!("meeting organization did not return the required function call");
    }
    let mut accepted = None;
    let mut last_invalid = None;
    for call in output.function_calls {
        if call.name != "save_meeting_artifact" {
            bail!("meeting organization returned unexpected function");
        }
        let artifact = match parse_meeting_artifact(&call.arguments) {
            Ok(artifact) => artifact,
            Err(error) => {
                last_invalid = Some(error);
                continue;
            }
        };
        if artifact.todos.iter().any(|todo| {
            matches!(
                (todo.source_start_ms, todo.source_end_ms),
                (Some(start), Some(end)) if start < timeline_start || end > timeline_end
            )
        }) {
            last_invalid = Some(anyhow::anyhow!(
                "meeting todo source range is outside the transcript timeline"
            ));
            continue;
        }
        if accepted
            .as_ref()
            .is_some_and(|accepted| accepted != &artifact)
        {
            bail!("meeting organization returned conflicting repeated function calls");
        }
        accepted = Some(artifact);
    }
    if let Some(artifact) = accepted {
        return Ok(artifact);
    }
    Err(last_invalid.unwrap_or_else(|| {
        anyhow::anyhow!("meeting organization did not return a usable artifact")
    }))
}

pub fn meeting_organization_sections(
    segments: &[TranscriptSegment],
    maximum_chars: usize,
) -> anyhow::Result<Vec<String>> {
    if maximum_chars == 0 || segments.iter().any(|segment| segment.provisional) {
        bail!("invalid meeting organization input");
    }
    let mut sections = Vec::new();
    let mut current = String::new();
    for segment in segments {
        if segment.start_ms < 0 || segment.end_ms <= segment.start_ms {
            bail!("invalid meeting transcript timing");
        }
        let prefix = format!("[{}-{}] ", segment.start_ms, segment.end_ms);
        let available = maximum_chars
            .checked_sub(prefix.chars().count())
            .filter(|available| *available > 0)
            .context("meeting organization character bound is too small")?;
        let chars = segment.text.trim().chars().collect::<Vec<_>>();
        if chars.is_empty() {
            continue;
        }
        for chunk in chars.chunks(available) {
            let line = format!("{}{}", prefix, chunk.iter().collect::<String>());
            let separator = usize::from(!current.is_empty());
            if !current.is_empty()
                && current.chars().count() + separator + line.chars().count() > maximum_chars
            {
                sections.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&line);
        }
    }
    if !current.is_empty() {
        sections.push(current);
    }
    Ok(sections)
}

fn organization_summary_batches(
    summaries: &[String],
    maximum_chars: usize,
) -> anyhow::Result<Vec<Vec<String>>> {
    if maximum_chars == 0 {
        bail!("meeting organization character bound must be positive");
    }
    let mut batches = Vec::<Vec<String>>::new();
    let mut current = Vec::<String>::new();
    let mut current_chars = 0_usize;
    for summary in summaries {
        let summary_chars = summary.chars().count();
        if summary_chars > maximum_chars {
            bail!("meeting summary item exceeded the organization input limit");
        }
        let separator = usize::from(!current.is_empty()) * 2;
        if !current.is_empty() && current_chars + separator + summary_chars > maximum_chars {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        if !current.is_empty() {
            current_chars += 2;
        }
        current.push(summary.clone());
        current_chars += summary_chars;
    }
    if !current.is_empty() {
        batches.push(current);
    }
    Ok(batches)
}

pub fn parse_meeting_artifact(arguments: &str) -> anyhow::Result<MeetingArtifact> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RawArtifact {
        title: String,
        summary: String,
        todos: Vec<RawTodo>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RawTodo {
        text: String,
        source_start_ms: serde_json::Value,
        source_end_ms: serde_json::Value,
    }

    let raw: RawArtifact =
        serde_json::from_str(arguments).context("meeting artifact arguments were invalid JSON")?;
    let artifact = MeetingArtifact {
        title: raw.title,
        summary: raw.summary,
        todos: raw
            .todos
            .into_iter()
            .map(|todo| {
                Ok(MeetingTodoDraft {
                    text: todo.text,
                    source_start_ms: required_nullable_i64(todo.source_start_ms)?,
                    source_end_ms: required_nullable_i64(todo.source_end_ms)?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
    };
    if artifact.title.trim().is_empty()
        || artifact.summary.trim().is_empty()
        || artifact.todos.len() > 50
    {
        bail!("meeting artifact title, summary, or todo count is invalid");
    }
    for MeetingTodoDraft {
        text,
        source_start_ms,
        source_end_ms,
    } in &artifact.todos
    {
        if text.trim().is_empty() {
            bail!("meeting todo text is empty");
        }
        match (*source_start_ms, *source_end_ms) {
            (None, None) => {}
            (Some(start), Some(end)) if start >= 0 && end > start => {}
            _ => bail!("meeting todo source range is invalid"),
        }
    }
    Ok(MeetingArtifact {
        title: artifact.title.trim().to_owned(),
        summary: artifact.summary.trim().to_owned(),
        todos: artifact
            .todos
            .into_iter()
            .map(|todo| MeetingTodoDraft {
                text: todo.text.trim().to_owned(),
                ..todo
            })
            .collect(),
    })
}

fn required_nullable_i64(value: serde_json::Value) -> anyhow::Result<Option<i64>> {
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_i64()
            .map(Some)
            .context("meeting todo source range must be an integer or null")
    }
}

fn samples_to_ms(samples: usize) -> i64 {
    (samples as i64) * 1_000 / PCM_SAMPLE_RATE as i64
}

#[cfg(test)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineWindow {
    media_start_ms: i64,
    start_ms: i64,
    end_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContinuityGroup {
    media_start_ms: i64,
    start_ms: i64,
    end_ms: i64,
}

fn continuity_groups(chunks: &[StoredChunkMetadata]) -> Vec<ContinuityGroup> {
    let mut groups = Vec::<ContinuityGroup>::new();
    let mut media_cursor_ms = 0_i64;
    for chunk in chunks {
        let duration_ms = chunk.end_ms - chunk.start_ms;
        if let Some(group) = groups.last_mut()
            && group.end_ms == chunk.start_ms
        {
            group.end_ms = chunk.end_ms;
        } else {
            groups.push(ContinuityGroup {
                media_start_ms: media_cursor_ms,
                start_ms: chunk.start_ms,
                end_ms: chunk.end_ms,
            });
        }
        media_cursor_ms += duration_ms;
    }
    groups
}

fn continuity_windows(
    groups: &[ContinuityGroup],
    window_ms: i64,
    overlap_ms: i64,
) -> Vec<TimelineWindow> {
    if window_ms <= 0 || overlap_ms < 0 || overlap_ms >= window_ms {
        return Vec::new();
    }
    let mut windows = Vec::new();
    for group in groups {
        if group.start_ms < 0 || group.end_ms <= group.start_ms {
            continue;
        }
        let duration = group.end_ms - group.start_ms;
        let mut local_start = 0;
        loop {
            let local_end = (local_start + window_ms).min(duration);
            windows.push(TimelineWindow {
                media_start_ms: group.media_start_ms + local_start,
                start_ms: group.start_ms + local_start,
                end_ms: group.start_ms + local_end,
            });
            if local_end == duration {
                break;
            }
            local_start = local_end - overlap_ms;
        }
    }
    windows
}

fn validate_chunk_timeline(chunks: &[StoredChunkMetadata]) -> anyhow::Result<()> {
    for (index, chunk) in chunks.iter().enumerate() {
        if chunk.sequence != index as i64 || chunk.start_ms < 0 || chunk.end_ms <= chunk.start_ms {
            bail!("invalid meeting chunk timeline");
        }
        if let Some(previous) = index.checked_sub(1).and_then(|index| chunks.get(index))
            && previous.end_ms > chunk.start_ms
        {
            bail!("overlapping meeting chunks are not supported");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SpeechSpan {
    start_ms: i64,
    end_ms: i64,
}

fn detect_speech_spans(pcm: &[i16]) -> Vec<SpeechSpan> {
    let frame_samples = PCM_SAMPLE_RATE * VAD_FRAME_MS as usize / 1_000;
    if frame_samples == 0 || pcm.is_empty() {
        return Vec::new();
    }
    let voiced = pcm
        .chunks(frame_samples)
        .map(|frame| {
            let energy = frame
                .iter()
                .map(|sample| i64::from(sample.unsigned_abs()))
                .sum::<i64>()
                / frame.len().max(1) as i64;
            energy >= VAD_ENERGY_THRESHOLD
        })
        .collect::<Vec<_>>();
    let mut runs = Vec::<(usize, usize)>::new();
    let mut index = 0;
    while index < voiced.len() {
        if !voiced[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < voiced.len() && voiced[index] {
            index += 1;
        }
        runs.push((start, index));
    }
    let maximum_silence_frames = (VAD_MAX_SILENCE_MS / VAD_FRAME_MS) as usize;
    let mut merged = Vec::<(usize, usize)>::new();
    for run in runs {
        if let Some(previous) = merged.last_mut()
            && run.0.saturating_sub(previous.1) <= maximum_silence_frames
        {
            previous.1 = run.1;
            continue;
        }
        merged.push(run);
    }
    let minimum_frames = (VAD_MIN_SPEECH_MS / VAD_FRAME_MS) as usize;
    let maximum_frames = (VAD_MAX_SPAN_MS / VAD_FRAME_MS) as usize;
    let overlap_frames = (VAD_SPAN_OVERLAP_MS / VAD_FRAME_MS) as usize;
    let mut spans = Vec::new();
    for (start, end) in merged {
        if end.saturating_sub(start) < minimum_frames {
            continue;
        }
        let mut part_start = start;
        while part_start < end {
            let part_end = (part_start + maximum_frames).min(end);
            spans.push(SpeechSpan {
                start_ms: part_start as i64 * VAD_FRAME_MS,
                end_ms: (part_end as i64 * VAD_FRAME_MS).min(samples_to_ms(pcm.len())),
            });
            if part_end == end {
                break;
            }
            part_start = part_end.saturating_sub(overlap_frames);
        }
    }
    spans
}

fn transcription_spans(pcm: &[i16]) -> Vec<SpeechSpan> {
    let detected = detect_speech_spans(pcm);
    if !detected.is_empty() || pcm.is_empty() {
        return detected;
    }
    let energy = pcm
        .iter()
        .map(|sample| i64::from(sample.unsigned_abs()))
        .sum::<i64>()
        / pcm.len() as i64;
    if energy < FALLBACK_NON_SILENCE_ENERGY {
        return Vec::new();
    }
    let duration_ms = samples_to_ms(pcm.len());
    (0..duration_ms)
        .step_by(FALLBACK_SPAN_MS as usize)
        .map(|start_ms| SpeechSpan {
            start_ms,
            end_ms: (start_ms + FALLBACK_SPAN_MS).min(duration_ms),
        })
        .collect()
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
        if let Some(previous) = merged.last()
            && start_ms < previous.end_ms
        {
            let duplicate_bytes = boundary_overlap_bytes(&previous.text, adjusted_text);
            if duplicate_bytes > 0 {
                adjusted_text = &adjusted_text[duplicate_bytes..];
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

fn boundary_overlap_bytes(left: &str, right: &str) -> usize {
    if left.is_ascii() && right.is_ascii() {
        let left_words = ascii_words(left);
        let right_words = ascii_words(right);
        let maximum = left_words.len().min(right_words.len());
        for length in (1..=maximum).rev() {
            let suffix = &left_words[left_words.len() - length..];
            let prefix = &right_words[..length];
            if suffix
                .iter()
                .map(|word| &word.0)
                .eq(prefix.iter().map(|word| &word.0))
            {
                return prefix.last().map(|word| word.2).unwrap_or(0);
            }
        }
        return 0;
    }
    let duplicate_chars = longest_boundary_overlap(left, right);
    if duplicate_chars < 2 {
        0
    } else {
        char_byte_offset(right, duplicate_chars)
    }
}

fn ascii_words(value: &str) -> Vec<(String, usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    for (offset, character) in value.char_indices() {
        if character.is_ascii_alphanumeric() || character == '\'' {
            start.get_or_insert(offset);
        } else if let Some(start) = start.take() {
            words.push((value[start..offset].to_ascii_lowercase(), start, offset));
        }
    }
    if let Some(start) = start {
        words.push((value[start..].to_ascii_lowercase(), start, value.len()));
    }
    words
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

    use super::{
        ContinuityGroup, MeetingProcessor, ORGANIZATION_SECTION_CHARS, continuity_windows,
        detect_speech_spans, meeting_organization_sections, parse_meeting_artifact,
        reconcile_overlap, validated_meeting_artifact, window_ranges,
    };
    use crate::{
        adapters::ModelAdapters,
        config::Settings,
        context::ContextStore,
        meeting::{storage::MeetingStorage, store::MeetingStore, types::TranscriptSegment},
    };
    use sha2::{Digest, Sha256};
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
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            "/usr/bin/ffmpeg".into(),
        );
        let relative_path = format!("{}/chunks/0.m4a", meeting.id);
        let absolute_path = directory.path().join("audio").join(&relative_path);
        tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
        generate_m4a(&absolute_path, 0.36).await?;
        let bytes = tokio::fs::read(&absolute_path).await?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        store
            .record_verified_chunk(
                &meeting.id,
                0,
                0,
                360,
                &checksum,
                bytes.len() as i64,
                &relative_path,
            )
            .await?;
        store.enqueue_transcript_job(&meeting.id).await?;

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
        let processor = processor.with_retry_delays_for_test(vec![Duration::ZERO; 2]);
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

    async fn generated_meeting_with_chunks(
        durations_ms: &[i64],
        starts_ms: &[i64],
        results: Vec<Result<&str, &str>>,
        ffmpeg_bin: &Path,
    ) -> anyhow::Result<(
        tempfile::TempDir,
        MeetingStore,
        MeetingStorage,
        MeetingProcessor,
        crate::adapters::TranscriptionTestProbe,
        String,
        String,
    )> {
        anyhow::ensure!(durations_ms.len() == starts_ms.len());
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store
            .create_with_status("user-a", "multi-chunk", 1.0)
            .await?;
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            ffmpeg_bin.to_path_buf(),
        );
        for (sequence, (&duration_ms, &start_ms)) in durations_ms.iter().zip(starts_ms).enumerate()
        {
            let relative_path = format!("{}/chunks/{sequence}.m4a", meeting.id);
            store
                .record_verified_chunk(
                    &meeting.id,
                    sequence as i64,
                    start_ms,
                    start_ms + duration_ms,
                    &format!("{sequence:064x}"),
                    1,
                    &relative_path,
                )
                .await?;
        }
        store.enqueue_transcript_job(&meeting.id).await?;
        let settings = Settings::from_env()?;
        let (adapters, probe) = ModelAdapters::with_transcription_test_results(
            settings,
            results
                .into_iter()
                .map(|result| result.map(str::to_owned).map_err(str::to_owned))
                .collect(),
            Duration::ZERO,
        )?;
        let processor = MeetingProcessor::new(store.clone(), storage.clone(), adapters, 1)?
            .with_retry_delays_for_test(vec![Duration::ZERO; 2]);
        Ok((
            directory,
            store,
            storage,
            processor,
            probe,
            meeting.id.clone(),
            format!("{}/recording.m4a", meeting.id),
        ))
    }

    async fn voiced_decoder(path: &Path) -> anyhow::Result<()> {
        tokio::fs::write(
            path,
            "#!/usr/bin/python3\nimport sys\nargs=sys.argv\nduration=float(args[args.index('-t')+1])\nsys.stdout.buffer.write(b'\\xa0\\x0f' * int(duration * 16000))\n",
        )
        .await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = tokio::fs::metadata(path).await?.permissions();
            permissions.set_mode(0o755);
            tokio::fs::set_permissions(path, permissions).await?;
        }
        Ok(())
    }

    async fn silent_decoder(path: &Path) -> anyhow::Result<()> {
        tokio::fs::write(
            path,
            "#!/usr/bin/python3\nimport sys\nargs=sys.argv\nduration=float(args[args.index('-t')+1])\nsys.stdout.buffer.write(b'\\x00\\x00' * int(duration * 16000))\n",
        )
        .await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = tokio::fs::metadata(path).await?.permissions();
            permissions.set_mode(0o755);
            tokio::fs::set_permissions(path, permissions).await?;
        }
        Ok(())
    }

    async fn low_volume_decoder(path: &Path) -> anyhow::Result<()> {
        tokio::fs::write(
            path,
            "#!/usr/bin/python3\nimport sys\nargs=sys.argv\nduration=float(args[args.index('-t')+1])\nsys.stdout.buffer.write(b'\\x64\\x00' * int(duration * 16000))\n",
        )
        .await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = tokio::fs::metadata(path).await?.permissions();
            permissions.set_mode(0o755);
            tokio::fs::set_permissions(path, permissions).await?;
        }
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

    #[test]
    fn preserves_declared_chunk_gaps_in_global_window_timeline() {
        let windows = continuity_windows(
            &[
                ContinuityGroup {
                    media_start_ms: 0,
                    start_ms: 0,
                    end_ms: 15_000,
                },
                ContinuityGroup {
                    media_start_ms: 15_000,
                    start_ms: 30_000,
                    end_ms: 45_000,
                },
            ],
            60_000,
            1_000,
        );

        assert!(windows.iter().any(|window| window.end_ms == 15_000));
        assert!(windows.iter().any(|window| window.start_ms == 30_000));
        assert!(
            !windows
                .iter()
                .any(|window| (15_000..30_000).contains(&window.start_ms))
        );
        assert!(
            windows
                .iter()
                .all(|window| window.end_ms - window.start_ms <= 60_000)
        );
    }

    #[test]
    fn energy_vad_skips_silence_and_locates_speech_with_frame_precision() {
        let mut pcm = vec![0_i16; 45 * 16_000];
        pcm.extend(vec![4_000_i16; 2 * 16_000]);

        let spans = detect_speech_spans(&pcm);

        assert_eq!(spans.len(), 1);
        assert!((44_980..=45_020).contains(&spans[0].start_ms));
        assert!((46_980..=47_020).contains(&spans[0].end_ms));
        assert!(detect_speech_spans(&vec![0_i16; 60 * 16_000]).is_empty());
    }

    #[test]
    fn energy_vad_merges_short_pauses_and_splits_long_speech() {
        let mut short_pause = vec![4_000_i16; 2 * 16_000];
        short_pause.extend(vec![0_i16; 300 * 16]);
        short_pause.extend(vec![4_000_i16; 2 * 16_000]);
        assert_eq!(detect_speech_spans(&short_pause).len(), 1);

        let continuous = vec![4_000_i16; 45 * 16_000];
        let spans = detect_speech_spans(&continuous);
        assert_eq!(spans.len(), 3);
        assert_eq!(
            spans,
            vec![
                super::SpeechSpan {
                    start_ms: 0,
                    end_ms: 20_000,
                },
                super::SpeechSpan {
                    start_ms: 19_000,
                    end_ms: 39_000,
                },
                super::SpeechSpan {
                    start_ms: 38_000,
                    end_ms: 45_000,
                },
            ]
        );
        assert!(
            spans
                .iter()
                .all(|span| span.end_ms - span.start_ms <= 20_000)
        );
    }

    #[tokio::test]
    async fn finalization_decodes_across_real_contiguous_m4a_boundaries_and_reconciles_overlap()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store
            .create_with_status("user-a", "real-boundary", 1.0)
            .await?;
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            "/usr/bin/ffmpeg".into(),
        );
        let mut paths = Vec::new();
        for sequence in 0..2 {
            let relative_path = format!("{}/chunks/{sequence}.m4a", meeting.id);
            let absolute_path = directory.path().join("audio").join(&relative_path);
            tokio::fs::create_dir_all(absolute_path.parent().unwrap()).await?;
            generate_m4a(&absolute_path, 1.2).await?;
            let bytes = tokio::fs::read(&absolute_path).await?;
            store
                .record_verified_chunk(
                    &meeting.id,
                    sequence,
                    sequence * 1_200,
                    sequence * 1_200 + 1_200,
                    &format!("{:x}", Sha256::digest(&bytes)),
                    bytes.len() as i64,
                    &relative_path,
                )
                .await?;
            paths.push(relative_path);
        }
        let final_audio = storage.assemble_final_audio(&meeting.id, &paths).await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let (adapters, probe) = ModelAdapters::with_transcription_test_results(
            Settings::from_env()?,
            vec![
                Ok("alpha boundary".to_owned()),
                Ok("boundary omega".to_owned()),
            ],
            Duration::ZERO,
        )?;
        let processor = MeetingProcessor::new(store, storage, adapters, 1)?
            .with_window_for_test(1_500, 500)
            .with_retry_delays_for_test(vec![Duration::ZERO; 2]);

        let segments = processor
            .finalize_transcript(&meeting.id, &final_audio.relative_path)
            .await?
            .unwrap();

        assert_eq!(probe.attempts(), 2);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "alpha boundary");
        assert_eq!(segments[1].text, "omega");
        assert_eq!(segments[0].end_ms, 1_500);
        assert_eq!(segments[1].start_ms, 1_500);
        assert_eq!(segments[1].end_ms, 2_400);
        Ok(())
    }

    #[tokio::test]
    async fn finalization_keeps_real_declared_gaps_out_of_continuity_groups() -> anyhow::Result<()>
    {
        let directory = tempfile::tempdir()?;
        let decoder = directory.path().join("voiced-decoder.py");
        voiced_decoder(&decoder).await?;
        let (_files, _store, _storage, processor, probe, meeting_id, final_path) =
            generated_meeting_with_chunks(
                &[1_200, 1_200],
                &[0, 3_000],
                vec![Ok("first"), Ok("second")],
                &decoder,
            )
            .await?;

        let segments = processor
            .finalize_transcript(&meeting_id, &final_path)
            .await?
            .unwrap();

        assert_eq!(probe.attempts(), 2);
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 1_200));
        assert_eq!((segments[1].start_ms, segments[1].end_ms), (3_000, 4_200));
        Ok(())
    }

    #[tokio::test]
    async fn two_hundred_ms_contiguous_chunks_do_not_amplify_asr_calls() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let decoder = directory.path().join("voiced-decoder.py");
        voiced_decoder(&decoder).await?;
        let durations = vec![200; 100];
        let starts = (0..100).map(|index| index * 200).collect::<Vec<_>>();
        let (_files, _store, _storage, processor, probe, meeting_id, final_path) =
            generated_meeting_with_chunks(
                &durations,
                &starts,
                vec![Ok("twenty seconds")],
                &decoder,
            )
            .await?;

        processor
            .finalize_transcript(&meeting_id, &final_path)
            .await?;

        assert_eq!(probe.attempts(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn silent_contiguous_processor_input_makes_zero_asr_calls() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let decoder = directory.path().join("silent-decoder.py");
        silent_decoder(&decoder).await?;
        let durations = vec![200; 100];
        let starts = (0..100).map(|index| index * 200).collect::<Vec<_>>();
        let (_files, _store, _storage, processor, probe, meeting_id, final_path) =
            generated_meeting_with_chunks(&durations, &starts, Vec::new(), &decoder).await?;

        let segments = processor
            .finalize_transcript(&meeting_id, &final_path)
            .await?
            .unwrap();

        assert_eq!(probe.attempts(), 0);
        assert!(segments.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn finalization_falls_back_for_low_volume_non_silent_audio() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let decoder = directory.path().join("low-volume-decoder.py");
        low_volume_decoder(&decoder).await?;
        let (_files, _store, _storage, processor, probe, meeting_id, final_path) =
            generated_meeting_with_chunks(
                &[12_000],
                &[0],
                vec![Ok("first low voice"), Ok("second low voice")],
                &decoder,
            )
            .await?;

        let segments = processor
            .finalize_transcript(&meeting_id, &final_path)
            .await?
            .unwrap();

        assert_eq!(probe.attempts(), 2);
        assert_eq!(segments.len(), 2);
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (0, 10_000));
        assert_eq!((segments[1].start_ms, segments[1].end_ms), (10_000, 12_000));
        assert_eq!(segments[0].text, "first low voice");
        assert_eq!(segments[1].text, "second low voice");
        Ok(())
    }

    #[tokio::test]
    async fn four_hour_continuous_processor_input_has_a_bounded_asr_call_count()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let decoder = directory.path().join("voiced-decoder.py");
        voiced_decoder(&decoder).await?;
        let durations = vec![60_000; 240];
        let starts = (0..240).map(|index| index * 60_000).collect::<Vec<_>>();
        let results = (0..758).map(|_| Ok("voice")).collect::<Vec<_>>();
        let (_files, _store, _storage, processor, probe, meeting_id, final_path) =
            generated_meeting_with_chunks(&durations, &starts, results, &decoder).await?;

        processor
            .finalize_transcript(&meeting_id, &final_path)
            .await?;

        assert_eq!(probe.attempts(), 758);
        Ok(())
    }

    #[test]
    fn overlap_deduplication_requires_time_overlap_and_token_boundaries() {
        let english = reconcile_overlap(&[
            (0, 1_000, "review the release plan"),
            (800, 1_800, "release plan and next step"),
        ]);
        assert_eq!(english[1].text, "and next step");

        let non_overlapping = reconcile_overlap(&[
            (0, 1_000, "release plan"),
            (1_500, 2_500, "release plan and next step"),
        ]);
        assert_eq!(non_overlapping[1].text, "release plan and next step");

        let word_prefix =
            reconcile_overlap(&[(0, 1_000, "project plan"), (800, 1_800, "planet next step")]);
        assert_eq!(word_prefix[1].text, "planet next step");
    }

    #[test]
    fn chinese_single_character_coincidence_and_silence_do_not_delete_text() {
        let merged = reconcile_overlap(&[
            (0, 1_000, "方案一"),
            (800, 1_800, "一起讨论"),
            (1_800, 2_800, ""),
            (3_000, 4_000, "一起讨论后续"),
        ]);

        assert_eq!(merged[1].text, "一起讨论");
        assert_eq!(merged[2].text, "一起讨论后续");
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
            .await?
            .unwrap();

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

        let first = processor
            .spawn_transcribe_chunk(meeting_id.clone(), 0, 0, 360, relative_path.clone())
            .unwrap();
        assert!(
            processor
                .spawn_transcribe_chunk(meeting_id.clone(), 0, 0, 360, relative_path.clone())
                .is_none()
        );
        first.await?;
        let retry = processor
            .spawn_transcribe_chunk(meeting_id, 0, 0, 360, relative_path)
            .expect("completed job must release its in-flight guard");
        retry.await?;

        assert_eq!(probe.attempts(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn failed_and_cancelled_chunk_jobs_release_guard_for_retry() -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, probe, meeting_id, relative_path) = fixture(
            vec![Err("one"), Err("two"), Err("three"), Ok("retry")],
            Duration::from_millis(20),
            1,
        )
        .await?;
        let failed = processor
            .spawn_transcribe_chunk(meeting_id.clone(), 0, 0, 360, relative_path.clone())
            .unwrap();
        failed.await?;
        let cancelled = processor
            .spawn_transcribe_chunk(meeting_id.clone(), 0, 0, 360, relative_path.clone())
            .unwrap();
        cancelled.abort();
        let _ = cancelled.await;
        let retry = processor
            .spawn_transcribe_chunk(meeting_id, 0, 0, 360, relative_path)
            .expect("cancelled job must release its in-flight guard");
        retry.await?;
        assert_eq!(probe.active(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn panicked_chunk_job_releases_guard_for_retry() -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, _probe, meeting_id, relative_path) =
            fixture(vec![Err("__panic__"), Ok("retry")], Duration::ZERO, 1).await?;
        let panicked = processor
            .spawn_transcribe_chunk(meeting_id.clone(), 0, 0, 360, relative_path.clone())
            .unwrap();
        assert!(panicked.await.is_err());
        let retry = processor
            .spawn_transcribe_chunk(meeting_id, 0, 0, 360, relative_path)
            .expect("panicked job must release its in-flight guard");
        retry.await?;
        Ok(())
    }

    #[tokio::test]
    async fn persistent_transcript_claim_makes_concurrent_finalize_single_flight()
    -> anyhow::Result<()> {
        let (_directory, _store, _storage, processor, probe, meeting_id, relative_path) = fixture(
            vec![Ok("今天讨论发布计划"), Ok("发布计划下周开始")],
            Duration::from_millis(20),
            2,
        )
        .await?;
        let processor = processor.with_window_for_test(250, 100);

        let first = processor.spawn_finalize_transcript(meeting_id.clone(), relative_path.clone());
        let second = processor.spawn_finalize_transcript(meeting_id, relative_path);
        first.await?;
        second.await?;

        assert_eq!(probe.attempts(), 2);
        Ok(())
    }

    #[test]
    fn meeting_organization_sections_preserve_ranges_and_character_bound() {
        let segments = vec![
            TranscriptSegment {
                id: 0,
                start_ms: 0,
                end_ms: 1_000,
                text: "alpha".to_owned(),
                provisional: false,
            },
            TranscriptSegment {
                id: 1,
                start_ms: 1_000,
                end_ms: 2_000,
                text: "beta".to_owned(),
                provisional: false,
            },
        ];

        let sections = meeting_organization_sections(&segments, 18).unwrap();

        assert_eq!(sections, vec!["[0-1000] alpha", "[1000-2000] beta"]);
        assert!(sections.iter().all(|section| section.chars().count() <= 18));
    }

    #[test]
    fn meeting_organization_rejects_invalid_structured_outputs() {
        assert!(parse_meeting_artifact("plain text").is_err());
        assert!(parse_meeting_artifact(r#"{"title":"","summary":"x","todos":[]}"#).is_err());
        assert!(
            parse_meeting_artifact(r#"{"title":"发布会","summary":"   ","todos":[]}"#).is_err()
        );
        assert!(parse_meeting_artifact(
            r#"{"title":"t","summary":"s","todos":[{"text":"x","source_start_ms":1,"source_end_ms":null}]}"#
        )
        .is_err());
        assert!(
            parse_meeting_artifact(r#"{"title":"t","summary":"s","todos":[{"text":"x"}]}"#)
                .is_err()
        );

        let too_many = serde_json::json!({
            "title": "t",
            "summary": "s",
            "todos": (0..51).map(|index| serde_json::json!({"text": format!("todo-{index}"), "source_start_ms": null, "source_end_ms": null})).collect::<Vec<_>>()
        });
        assert!(parse_meeting_artifact(&too_many.to_string()).is_err());
    }

    #[test]
    fn meeting_organization_rejects_conflicting_repeated_function_calls() {
        let output = crate::adapters::ResponsesOutput {
            text: String::new(),
            function_calls: vec![
                crate::adapters::FunctionCall {
                    call_id: "first".to_owned(),
                    name: "save_meeting_artifact".to_owned(),
                    arguments: r#"{"title":"标题一","summary":"摘要一","todos":[]}"#.to_owned(),
                },
                crate::adapters::FunctionCall {
                    call_id: "second".to_owned(),
                    name: "save_meeting_artifact".to_owned(),
                    arguments: r#"{"title":"标题二","summary":"摘要二","todos":[]}"#.to_owned(),
                },
            ],
            output_items: Vec::new(),
        };

        assert!(validated_meeting_artifact(output, 0, 1_000).is_err());
    }

    #[tokio::test]
    async fn meeting_organization_uses_forced_function_and_never_global_todos() -> anyhow::Result<()>
    {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store
            .create_with_status("user-a", "meeting-organization", 1.0)
            .await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        store
            .complete_transcript_job(
                &meeting.id,
                attempt,
                &[TranscriptSegment {
                    id: 0,
                    start_ms: 0,
                    end_ms: 2_000,
                    text: "下周发布，张三准备验收清单".to_owned(),
                    provisional: false,
                }],
            )
            .await?;
        let (adapters, probe) = ModelAdapters::with_meeting_organization_test_results(
            Settings::from_env()?,
            vec![
                Ok(crate::adapters::ResponsesOutput {
                    text: "先输出了一次非结构化结果。".to_owned(),
                    function_calls: Vec::new(),
                    output_items: Vec::new(),
                }),
                Ok(crate::adapters::ResponsesOutput {
                    text: "会议内容已整理，正在保存结构化结果。".to_owned(),
                    function_calls: vec![
                        crate::adapters::FunctionCall {
                            call_id: "call-incomplete".to_owned(),
                            name: "save_meeting_artifact".to_owned(),
                            arguments: r#"{"summary":"中间结果","todos":[]}"#.to_owned(),
                        },
                        crate::adapters::FunctionCall {
                            call_id: "call-1".to_owned(),
                            name: "save_meeting_artifact".to_owned(),
                            arguments: r#"{"title":"发布准备会","summary":"确认下周发布。","todos":[{"text":"张三准备验收清单","source_start_ms":0,"source_end_ms":2000}]}"#.to_owned(),
                        },
                        crate::adapters::FunctionCall {
                            call_id: "call-1-duplicate".to_owned(),
                            name: "save_meeting_artifact".to_owned(),
                            arguments: r#"{"title":"发布准备会","summary":"确认下周发布。","todos":[{"text":"张三准备验收清单","source_start_ms":0,"source_end_ms":2000}]}"#.to_owned(),
                        },
                    ],
                    output_items: Vec::new(),
                }),
            ],
        )?;
        let storage = MeetingStorage::new(
            directory.path().join("audio"),
            2 * 1024 * 1024,
            "/usr/bin/ffmpeg".into(),
        );
        let processor = MeetingProcessor::new(store.clone(), storage, adapters, 1)?;

        let artifact = processor.organize_meeting(&meeting.id).await?.unwrap();

        assert_eq!(artifact.title, "发布准备会");
        let mut requests = probe.requests();
        assert_eq!(requests.len(), 2);
        let request = requests.pop().unwrap();
        assert_eq!(request["reasoning"]["effort"], "none");
        assert_eq!(request["tools"][0]["name"], "save_meeting_artifact");
        assert_eq!(
            request["tool_choice"],
            serde_json::json!({"type":"function","name":"save_meeting_artifact"})
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM todos")
                .fetch_one(&context.pool_clone())
                .await?,
            0
        );
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.title.as_deref(), Some("发布准备会"));
        assert_eq!(detail.todos.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn meeting_organization_rejects_todo_range_outside_final_transcript() -> anyhow::Result<()>
    {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store
            .create_with_status("user-a", "meeting-organization-range", 1.0)
            .await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        store
            .complete_transcript_job(
                &meeting.id,
                attempt,
                &[TranscriptSegment {
                    id: 0,
                    start_ms: 0,
                    end_ms: 2_000,
                    text: "下周发布".to_owned(),
                    provisional: false,
                }],
            )
            .await?;
        let (adapters, _) = ModelAdapters::with_meeting_organization_test_results(
            Settings::from_env()?,
            vec![Ok(crate::adapters::ResponsesOutput {
                text: String::new(),
                function_calls: vec![crate::adapters::FunctionCall {
                    call_id: "call-range".to_owned(),
                    name: "save_meeting_artifact".to_owned(),
                    arguments: r#"{"title":"发布会","summary":"发布","todos":[{"text":"准备发布","source_start_ms":9000,"source_end_ms":10000}]}"#.to_owned(),
                }],
                output_items: Vec::new(),
            })],
        )?;
        let processor = MeetingProcessor::new(
            store.clone(),
            MeetingStorage::new(
                directory.path().join("audio"),
                2 * 1024 * 1024,
                "/usr/bin/ffmpeg".into(),
            ),
            adapters,
            1,
        )?;

        assert!(processor.organize_meeting(&meeting.id).await.is_err());
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.title, None);
        assert_eq!(
            detail.error_stage,
            Some(crate::meeting::types::ProcessingStage::Organization)
        );
        assert_eq!(detail.transcript.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn meeting_organization_summarizes_bounded_sections_before_final_call()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store.create_with_status("user-a", "hierarchy", 1.0).await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        let segments = (0..13)
            .map(|index| TranscriptSegment {
                id: index,
                start_ms: index * 1_000,
                end_ms: index * 1_000 + 1_000,
                text: "会".repeat(11_900),
                provisional: false,
            })
            .collect::<Vec<_>>();
        store
            .complete_transcript_job(&meeting.id, attempt, &segments)
            .await?;
        let summary = |text: &str| crate::adapters::ResponsesOutput {
            text: text.to_owned(),
            function_calls: Vec::new(),
            output_items: Vec::new(),
        };
        let final_output = crate::adapters::ResponsesOutput {
            text: String::new(),
            function_calls: vec![crate::adapters::FunctionCall {
                call_id: "final".to_owned(),
                name: "save_meeting_artifact".to_owned(),
                arguments: r#"{"title":"长会","summary":"分段完成","todos":[]}"#.to_owned(),
            }],
            output_items: Vec::new(),
        };
        let mut outputs = (0..13)
            .map(|_| Ok(summary(&"摘要".repeat(512))))
            .collect::<Vec<_>>();
        outputs.push(Ok(summary("第一批压缩摘要")));
        outputs.push(Ok(summary("第二批压缩摘要")));
        outputs.push(Ok(final_output));
        let (adapters, probe) =
            ModelAdapters::with_meeting_organization_test_results(Settings::from_env()?, outputs)?;
        let processor = MeetingProcessor::new(
            store,
            MeetingStorage::new(
                directory.path().join("audio"),
                2 * 1024 * 1024,
                "/usr/bin/ffmpeg".into(),
            ),
            adapters,
            1,
        )?;

        processor.organize_meeting(&meeting.id).await?;

        let requests = probe.requests();
        assert_eq!(requests.len(), 16);
        for request in &requests {
            let input = request["input"][0]["content"][0]["text"].as_str().unwrap();
            assert!(
                input.chars().count() <= ORGANIZATION_SECTION_CHARS,
                "request exceeded hard cap"
            );
        }
        for request in &requests[..15] {
            assert_eq!(request["tool_choice"], "none");
            assert_eq!(request["tools"], serde_json::json!([]));
        }
        assert_eq!(requests[15]["tools"][0]["strict"], true);
        assert_eq!(
            requests[15]["tools"][0]["parameters"]["required"],
            serde_json::json!(["title", "summary", "todos"])
        );
        assert_eq!(
            requests[15]["tools"][0]["parameters"]["properties"]["summary"]["minLength"],
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn meeting_organization_stops_when_recursive_summary_does_not_shrink()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store.create_with_status("user-a", "no-shrink", 1.0).await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        store
            .complete_transcript_job(
                &meeting.id,
                attempt,
                &[TranscriptSegment {
                    id: 0,
                    start_ms: 0,
                    end_ms: 10_000,
                    text: "会".repeat(300),
                    provisional: false,
                }],
            )
            .await?;
        let summary = |text: String| crate::adapters::ResponsesOutput {
            text,
            function_calls: Vec::new(),
            output_items: Vec::new(),
        };
        let (adapters, probe) = ModelAdapters::with_meeting_organization_test_results(
            Settings::from_env()?,
            vec![
                Ok(summary("甲".repeat(40))),
                Ok(summary("乙".repeat(40))),
                Ok(summary("丙".repeat(40))),
                Ok(summary("坏".repeat(120))),
            ],
        )?;
        let processor = MeetingProcessor::new(
            store.clone(),
            MeetingStorage::new(
                directory.path().join("audio"),
                2 * 1024 * 1024,
                "/usr/bin/ffmpeg".into(),
            ),
            adapters,
            1,
        )?
        .with_organization_input_chars_for_test(120);

        assert!(processor.organize_meeting(&meeting.id).await.is_err());
        let requests = probe.requests();
        assert!(requests.iter().all(|request| {
            request["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 120
        }));
        assert_eq!(requests.len(), 4);
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.title, None);
        assert_eq!(
            detail.error_stage,
            Some(crate::meeting::types::ProcessingStage::Organization)
        );
        Ok(())
    }

    #[tokio::test]
    async fn meeting_organization_handles_silence_without_provider_call() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store.create_with_status("user-a", "silence", 1.0).await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        store
            .complete_transcript_job(&meeting.id, attempt, &[])
            .await?;
        let (adapters, probe) = ModelAdapters::with_meeting_organization_test_results(
            Settings::from_env()?,
            Vec::new(),
        )?;
        let processor = MeetingProcessor::new(
            store.clone(),
            MeetingStorage::new(
                directory.path().join("audio"),
                2 * 1024 * 1024,
                "/usr/bin/ffmpeg".into(),
            ),
            adapters,
            1,
        )?;

        let artifact = processor.organize_meeting(&meeting.id).await?.unwrap();

        assert_eq!(artifact.title, "未检测到语音内容");
        assert!(probe.requests().is_empty());
        let detail = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(detail.state, crate::meeting::types::MeetingState::Completed);
        Ok(())
    }

    #[tokio::test]
    async fn failed_meeting_organization_is_retryable_without_losing_final_transcript()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let context = ContextStore::open(&directory.path().join("meeting.sqlite3")).await?;
        let store = MeetingStore::new(context.pool_clone());
        store.initialize().await?;
        let (meeting, _) = store.create_with_status("user-a", "org-retry", 1.0).await?;
        store.enqueue_transcript_job(&meeting.id).await?;
        let crate::meeting::types::TranscriptJobClaim::Claimed { attempt } =
            store.claim_transcript_job(&meeting.id).await?
        else {
            anyhow::bail!("transcript job was not claimable");
        };
        let transcript = vec![TranscriptSegment {
            id: 0,
            start_ms: 0,
            end_ms: 1_000,
            text: "确认发布".to_owned(),
            provisional: false,
        }];
        store
            .complete_transcript_job(&meeting.id, attempt, &transcript)
            .await?;
        let (adapters, _) = ModelAdapters::with_meeting_organization_test_results(
            Settings::from_env()?,
            vec![
                Ok(crate::adapters::ResponsesOutput {
                    text: "plain text is forbidden".to_owned(),
                    function_calls: Vec::new(),
                    output_items: Vec::new(),
                }),
                Ok(crate::adapters::ResponsesOutput {
                    text: "plain text is still forbidden".to_owned(),
                    function_calls: Vec::new(),
                    output_items: Vec::new(),
                }),
                Ok(crate::adapters::ResponsesOutput {
                    text: "plain text remains forbidden".to_owned(),
                    function_calls: Vec::new(),
                    output_items: Vec::new(),
                }),
                Ok(crate::adapters::ResponsesOutput {
                    text: String::new(),
                    function_calls: vec![crate::adapters::FunctionCall {
                        call_id: "retry".to_owned(),
                        name: "save_meeting_artifact".to_owned(),
                        arguments: r#"{"title":"发布会","summary":"确认发布","todos":[]}"#
                            .to_owned(),
                    }],
                    output_items: Vec::new(),
                }),
            ],
        )?;
        let processor = MeetingProcessor::new(
            store.clone(),
            MeetingStorage::new(
                directory.path().join("audio"),
                2 * 1024 * 1024,
                "/usr/bin/ffmpeg".into(),
            ),
            adapters,
            1,
        )?;

        assert!(processor.organize_meeting(&meeting.id).await.is_err());
        let failed = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(failed.transcript, transcript);
        assert_eq!(
            failed.error_message.as_deref(),
            Some("meeting organization failed")
        );
        assert_eq!(
            store
                .retry_stage_owned(
                    "user-a",
                    &meeting.id,
                    crate::meeting::types::ProcessingStage::Organization,
                )
                .await?,
            crate::meeting::types::RetryStageOutcome::Queued
        );
        processor.organize_meeting(&meeting.id).await?;
        let recovered = store.get_owned("user-a", &meeting.id).await?.unwrap();
        assert_eq!(recovered.title.as_deref(), Some("发布会"));
        assert_eq!(recovered.transcript, transcript);
        assert_eq!(recovered.error_stage, None);
        Ok(())
    }
}
