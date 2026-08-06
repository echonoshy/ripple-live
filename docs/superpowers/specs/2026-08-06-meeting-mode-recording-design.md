# Ripple Live Meeting Mode Recording Design

**Date:** 2026-08-06

**Status:** Approved for implementation planning

## 1. Objective

Add a manually started Meeting Mode to Ripple Live for reliable long-form audio recording and meeting organization.

Meeting Mode records audio and creates a time-aligned transcript. After the meeting ends, it produces a title, summary, and meeting-local action items. It does not answer questions, call tools, synthesize speech, capture video, or alter the existing voice/video Agent flows.

The first release targets Android. Existing iOS code remains frozen.

## 2. Confirmed Product Decisions

- Meeting Mode is a separate product entry, not a flag inside voice chat.
- The meeting shows provisional transcript text while recording.
- After recording ends, the service regenerates and corrects the final transcript from the complete audio.
- The first release does not distinguish speakers.
- Android must continue recording while the screen is locked or the App is in the background.
- Recording is local-first. Network loss can delay transcription and upload but cannot stop recording.
- Missing chunks upload automatically after connectivity returns.
- An interrupted or crashed App can recover an unfinished meeting.
- Transcript segments and audio share one timeline. Users can jump from text to audio, and playback highlights the current text.
- Extracted action items belong only to their meeting. They do not enter the existing global todo center.
- Four hours of continuous recording is the formal first-release reliability target. The product does not intentionally stop longer recordings, but durations beyond four hours are outside the first validation target.

## 3. Product Boundaries

Ripple Live keeps three independent content surfaces:

1. **Chat History:** current voice/video Agent conversations.
2. **Visual Memory:** current video-derived memories.
3. **Meeting Records:** long-form audio, transcript, summary, and meeting-local action items.

Meeting data must not be stored as realtime conversation turns or visual memories. The only future cross-surface action in scope for later consideration is an explicit user command to copy a meeting-local action item into the global todo center.

## 4. User Experience

### 4.1 Entry

The home screen adds a dedicated Meeting Record entry. It is visually and semantically separate from voice and video calls.

Starting a meeting opens a recording-specific screen. The header says `Meeting Mode - recording only, no replies` so the user cannot confuse it with the Agent.

### 4.2 Active Meeting Screen

The screen displays:

- elapsed recording time;
- recording, paused, offline, uploading, or recovery state;
- audio level;
- provisional transcript segments as they arrive;
- pause/resume and end controls;
- a clear offline message stating that recording continues and transcription will catch up later.

It does not display:

- Ripple assistant messages;
- thinking, tool, or speaking states;
- interrupt controls;
- camera controls;
- audio playback from TTS.

When the screen locks or the user switches Apps, an Android foreground-service notification remains visible and shows that Meeting Mode is recording. Returning to the App reopens the active meeting.

### 4.3 Ending and Processing

Ending a meeting closes the audio timeline permanently and moves the meeting through these visible stages:

1. upload missing chunks;
2. verify recording completeness;
3. generate the final transcript;
4. generate title and summary;
5. extract meeting-local action items;
6. complete.

The recording remains available even if later processing fails. A failed stage can be retried independently.

### 4.4 Meeting Center and Detail

Meeting Center is a separate library showing title, date, duration, and processing status.

The detail page contains:

- complete recording playback;
- final time-coded transcript;
- synchronized playback position and transcript highlight;
- click-to-seek from a transcript segment;
- title and summary;
- meeting-local action items and their completion states;
- upload/processing failure information and retry actions.

The playback/highlight alignment error must not exceed one second.

## 5. Architecture

Meeting Mode uses a dedicated local-first pipeline:

```text
Android foreground recorder
  -> encrypted local audio chunks and durable manifest
  -> idempotent upload queue
  -> server recording storage and provisional ASR
  -> final transcript reconstruction
  -> Responses API meeting organization
  -> Meeting Center and synchronized playback
```

The implementation must not reuse `LiveMedia` or `RealtimeSession` as the recording engine. Those components remain optimized for short VAD-bounded Agent turns and realtime response playback.

### 5.1 Android Recording Service

A native Android `MeetingRecorderService` owns microphone capture and foreground-service lifetime.

Responsibilities:

- start, pause, resume, and stop capture;
- continue capture while the React/Tauri UI is backgrounded;
- write an append-safe short PCM journal for the active chunk;
- finalize independent AAC/M4A chunks of approximately 10-15 seconds;
- record chunk sequence, start/end offsets, byte size, checksum, and state;
- encrypt local chunks with a key protected by Android Keystore;
- persist meeting and upload state after every transition;
- recover the active PCM journal after an unexpected process exit;
- report audio level, elapsed time, network state, and upload progress to the UI;
- handle microphone interruption and storage pressure safely.

The short PCM journal limits loss on an unexpected termination to the last unflushed audio buffer. Finalized chunks are immutable.

React controls the service through a focused Tauri Android plugin interface. The UI is a consumer of durable native state; it is not the owner of the recording lifecycle.

### 5.2 Local Persistence

Local storage contains:

- meeting metadata and lifecycle state;
- the active PCM journal;
- finalized encrypted audio chunks;
- chunk upload/checksum state;
- provisional transcript segments;
- a durable cursor for the last acknowledged server chunk.

Files remain in App-private storage. No local audio is removed until the server has acknowledged every chunk and verified the complete recording. After verification, local audio becomes a purgeable cache but remains retained by default in the first release.

### 5.3 Server API

Meeting resources are authenticated and owned by one user.

Initial endpoints:

- `POST /v1/meetings` creates a meeting and accepts an idempotency key.
- `PUT /v1/meetings/{meeting_id}/chunks/{sequence}` uploads one immutable chunk with offsets and checksum.
- `POST /v1/meetings/{meeting_id}/finalize` closes the timeline and starts asynchronous processing.
- `POST /v1/meetings/{meeting_id}/retry` retries a failed processing stage without duplicating completed work.
- `GET /v1/meetings` lists meetings.
- `GET /v1/meetings/{meeting_id}` returns details and processing progress.
- `GET /v1/meetings/{meeting_id}/audio` streams audio with time-range support.
- `DELETE /v1/meetings/{meeting_id}` deletes the meeting and all owned derivatives.

Chunk upload is idempotent by meeting ID, sequence, and checksum. A repeated matching upload succeeds without creating duplicates. A repeated sequence with a different checksum is a conflict and requires client reconciliation.

The server rejects finalization until all declared sequences are present. It reports missing sequences so the client can resume precisely.

### 5.4 Server Data Model

The server stores the following independent entities:

- `Meeting`: owner, state, start/end time, duration, title, summary, timestamps, and error state.
- `RecordingChunk`: meeting, sequence, time range, checksum, content path, and verification state.
- `TranscriptSegment`: meeting, sequence, start/end offsets, provisional text, final text, and revision state.
- `MeetingTodo`: meeting, text, completion state, source transcript time range, and timestamps.
- `MeetingProcessingJob`: meeting, stage, attempt, status, and diagnostic error.

Transcript/audio correspondence is defined by millisecond `start_ms` and `end_ms` offsets from the meeting timeline. It must not depend on array order alone.

## 6. Transcription and Organization

### 6.1 Provisional Transcript

The server transcribes uploaded chunks as they arrive. Results are provisional and may be replaced after finalization. Silence can produce no transcript segment while the audio chunk remains part of the authoritative recording.

Network loss pauses provisional transcription only. Once missing chunks arrive, the provisional timeline catches up.

### 6.2 Final Transcript

After finalization, the server verifies chunk continuity and performs ASR again across the complete timeline with small overlaps at chunk boundaries. It reconciles overlapping words and sentence boundaries while preserving audio offsets.

The language model does not rewrite quoted speech as if it were a corrected transcript. ASR output and its timing remain the source for the final transcript.

### 6.3 Title, Summary, and Action Items

The complete transcript can exceed one model context window, so organization uses bounded stages:

1. divide the final transcript into time-preserving sections;
2. summarize each section and extract candidate decisions/actions;
3. use the Responses API to synthesize a single structured result;
4. validate the structured output before writing title, summary, and meeting-local action items.

Action items remain attached to the meeting and include their source transcript time range when available. They are not automatically copied into the existing todo tables.

## 7. Lifecycle and Recovery

The primary lifecycle is:

```text
preparing -> recording <-> paused -> uploading -> processing -> completed
```

Errors are recorded against the current stage rather than replacing the meeting with an unrecoverable generic error.

Recovery rules:

- **Offline:** keep recording locally and retry uploads with bounded exponential backoff.
- **UI closed:** foreground service continues and durable native state remains authoritative.
- **Process terminated:** recover the meeting manifest and active journal when the App returns. Android force-stop cannot continue microphone capture, but the already recorded meeting remains recoverable and must be marked with the detected interruption.
- **Phone/audio-focus interruption:** pause capture, persist the interruption time, and notify the user. Never represent an interruption as successfully recorded silence.
- **Low storage:** warn before the safety threshold. At the threshold, finalize recoverable data and stop cleanly.
- **Duplicate upload/finalize:** use idempotent server operations.
- **ASR or organization failure:** preserve recording and completed transcript work, expose the failed stage, and retry only that stage.

Once the user ends a meeting, its audio timeline is immutable. Additional recording starts a new meeting.

## 8. Privacy and Security

- Meeting APIs require the existing authenticated user identity and enforce ownership on every resource.
- Local chunks are encrypted using an Android Keystore-protected key.
- Server audio is stored outside public static paths and is served only through authenticated handlers.
- Logs never contain audio bytes, transcript bodies, access tokens, or encryption keys.
- Deleting a meeting removes audio, transcript, title, summary, and meeting-local action items together.
- The recording screen and foreground notification always make recording state visible.
- HTTPS/WSS is a production-release requirement. The current cleartext transport is not acceptable for formal meeting-recording release outside a trusted test network.

## 9. Error Presentation

User-facing errors must distinguish:

- recording interrupted;
- microphone unavailable;
- low storage;
- offline but recording safely;
- upload incomplete;
- transcript processing failed;
- summary/action extraction failed.

Every recoverable failure provides the appropriate action: resume recording, free storage, retry upload, retry transcript, or retry organization. Generic `failed` status without stage and recovery guidance is not acceptable.

## 10. Verification

### 10.1 Automated Tests

- Android recorder lifecycle and persistent state transitions.
- Journal recovery and finalized-chunk immutability.
- Chunk checksum and encryption/decryption behavior.
- Upload queue ordering, retries, and acknowledgement cursor.
- Server authentication and cross-user isolation.
- Idempotent chunk upload, conflict detection, missing-sequence reporting, and finalization.
- Transcript overlap reconciliation and stable time ranges.
- Structured Responses API output validation.
- Meeting-local action items never appear in global todo APIs.
- Existing realtime voice/video and library tests continue to pass.

### 10.2 Android Device Tests

- Record continuously for four hours and play the complete result.
- Lock the screen and switch Apps repeatedly without interrupting capture.
- Remain offline for at least 30 minutes, reconnect, and verify automatic catch-up.
- Kill the UI while recording and verify foreground recording and recovery behavior.
- Interrupt microphone access and verify a visible paused/interrupted state.
- Exercise low-storage handling without corrupting completed chunks.
- Verify notification state, elapsed time, pause/resume, and return-to-meeting flow.

### 10.3 End-to-End Service Verification

Using an authenticated account and the configured Ripple service:

- create a meeting;
- upload real Android audio chunks;
- observe provisional transcript segments;
- finalize the meeting;
- verify complete audio and final transcript;
- issue a real Responses API organization request;
- verify title, summary, and meeting-local action items;
- verify click-to-seek and playback highlighting within one second;
- verify that no Agent reply, tool call, or TTS request occurs while recording.

Service health, a successful build, or isolated unit tests do not replace this end-to-end proof.

## 11. First Release Scope

Included:

- manual meeting start, pause, resume, and end;
- Android foreground/background and lock-screen recording;
- four-hour validated continuous recording;
- local-first encrypted chunks;
- offline continuation, resumable upload, and crash recovery;
- provisional and final transcripts;
- synchronized transcript/audio playback;
- independent Meeting Center;
- title, summary, and meeting-local action items;
- stage-specific processing retry.

Deferred:

- speaker diarization or real-name recognition;
- voice Q&A during a meeting;
- video meeting recording;
- global todo synchronization;
- collaboration and shared editing;
- calendar, Feishu, or external meeting-platform integration;
- iOS implementation;
- transcript editing, advanced templates, and exports.

## 12. Related Product Opportunities

The same recording foundation can later support separate, explicitly started workflows without mixing them into the realtime Agent:

1. **Interview record:** long-form recording, question/topic chapters, and quote review.
2. **Class or training notes:** timeline transcript, knowledge outline, and review points.
3. **Customer visit record:** needs, objections, commitments, and follow-up summary.
4. **Field inspection voice log:** hands-free observations tied to time and later structured into an inspection report; visual evidence remains in the separate Visual Memory surface.
5. **Personal voice journal:** manually started private recording with themes and daily summary.

These are future products built on the recording substrate. They do not expand the first Meeting Mode implementation scope.
