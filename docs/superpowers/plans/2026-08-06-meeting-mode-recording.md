# Meeting Mode Recording Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an Android-first, local-first Meeting Mode that records reliably for four hours, survives background/offline operation, synchronizes transcript and audio, and generates a meeting title, summary, and meeting-local action items.

**Architecture:** Implement three bounded subsystems in sequence: authenticated meeting resources and processing on the Rust Gateway, a native Android foreground recorder exposed through a Tauri mobile plugin, and React Meeting Center/live/detail screens. Audio is written to encrypted immutable local chunks before upload; the server accepts idempotent chunks, produces provisional/final ASR, and uses only the Responses API for title/summary/action organization.

**Tech Stack:** Rust 2024, Axum 0.8, SQLx/SQLite, Tokio, Reqwest, FFmpeg subprocess decoding, React 19, TypeScript 6, Tauri 2.11, Kotlin 1.9, Android AudioRecord/MediaCodec/MediaMuxer/Keystore/SQLiteOpenHelper, Node test runner, JUnit/Android instrumentation.

## Global Constraints

- Responses API is the only permitted Agent/model protocol; Meeting Mode must not introduce Chat Completions or another Agent protocol.
- Server implementation, verification, and deployment run on `lake@140.143.229.103`; mobile implementation and Android packaging run from `/Users/lake/workspace/ripple-live`.
- Android is the only mobile implementation target. Do not modify generated Apple files, Swift, iOS configuration, or iOS permissions.
- Meeting Mode is independent from realtime chat, Visual Memory, and the global todo center.
- No Agent reply, tool call, TTS, or camera capture may run during recording.
- Audio is authoritative and local-first. Upload or ASR failure must never discard finalized local chunks.
- Transcript/audio offsets use accumulated recorded-audio time; paused wall-clock intervals are excluded from `start_ms` and `end_ms`.
- Formal device validation is four continuous hours, including lock-screen/background operation and a 30-minute offline interval.
- Start the microphone foreground service only from a visible Activity after `RECORD_AUDIO` is granted; target SDK 36 requires the `microphone` service type plus `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_MICROPHONE` permissions.
- Keep access tokens, audio bytes, transcript bodies, and encryption keys out of logs.
- Production release requires HTTPS/WSS; cleartext remains debug-only until TLS is configured.

## Planned File Structure

### Server (`140.143.229.103`)

- `services/agent-gateway/src/meeting/types.rs`: serialized meeting domain/API types and lifecycle enums.
- `services/agent-gateway/src/meeting/store.rs`: meeting SQLite schema and ownership-scoped queries.
- `services/agent-gateway/src/meeting/storage.rs`: chunk paths, checksum verification, immutable writes, FFmpeg assembly/decoding, authenticated audio metadata.
- `services/agent-gateway/src/meeting/processor.rs`: provisional ASR, final transcript reconciliation, hierarchical Responses API organization, retries.
- `services/agent-gateway/src/meeting/mod.rs`: `MeetingService` facade and processing task recovery.
- `services/agent-gateway/src/meeting_api.rs`: Axum request parsing and authenticated meeting handlers.
- `services/agent-gateway/src/lib.rs`: exports meeting modules.
- `services/agent-gateway/src/main.rs`: constructs `MeetingService`, installs routes, and allows `PUT` in CORS.
- `services/agent-gateway/src/config.rs`: recording limits and explicit FFmpeg path.
- `services/agent-gateway/src/adapters.rs`: Responses API meeting organization and encoded-audio ASR helpers.
- `deploy/agent-stack/start.sh`, `status.sh`, `test-smoke-contract.py`: runtime configuration/readiness and meeting smoke contract.

### Android/Tauri (`/Users/lake/workspace/ripple-live`)

- `apps/mobile/src-tauri/src/meeting_recorder.rs`: Rust plugin registration, typed mobile commands, and Android-only guard.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingRecorderPlugin.kt`: Tauri command bridge and permission requests.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingRecorderService.kt`: foreground lifecycle, audio capture, notification, and upload coordinator.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingManifestStore.kt`: durable local meeting/chunk state.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingAudioCapture.kt`: PCM journal and AAC/M4A finalization.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingCrypto.kt`: Android Keystore AES-GCM encryption.
- `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingUploader.kt`: ordered idempotent upload and backoff.
- `apps/mobile/src-tauri/gen/android/app/src/test/java/cn/minicpm/live/`: JVM tests for state, storage naming, and upload cursor.
- `apps/mobile/src-tauri/gen/android/app/src/androidTest/java/cn/minicpm/live/MeetingRecorderInstrumentedTest.kt`: service/device lifecycle checks.
- `apps/mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml`: foreground permissions/service declaration.
- `apps/mobile/src-tauri/gen/android/app/build.gradle.kts`: Kotlin test/coroutine dependencies only where required.
- `apps/mobile/src-tauri/src/lib.rs`, `Cargo.toml`, `capabilities/default.json`: plugin registration and capability.

### React Mobile

- `apps/mobile/src/meetings/types.ts`: UI-facing meeting types and state labels.
- `apps/mobile/src/meetings/api.ts`: authenticated meeting CRUD, audio fetch, and polling.
- `apps/mobile/src/meetings/recorder.ts`: typed Tauri plugin wrapper and event subscription.
- `apps/mobile/src/meetings/timeline.ts`: pure transcript lookup/highlight functions.
- `apps/mobile/src/components/MeetingLive.tsx`: active recording screen.
- `apps/mobile/src/components/MeetingCenter.tsx`: independent meeting library.
- `apps/mobile/src/components/MeetingDetail.tsx`: audio, transcript, summary, and local action items.
- `apps/mobile/src/App.tsx`, `App.css`: navigation and scoped styling.
- `apps/mobile/tests/meeting-timeline.test.ts`, `meeting-api.test.ts`, `mobile-package.test.mjs`: behavior and package assertions.

---

### Task 1: Server Meeting Domain and Durable Store

**Files:**
- Create: `services/agent-gateway/src/meeting/types.rs`
- Create: `services/agent-gateway/src/meeting/store.rs`
- Create: `services/agent-gateway/src/meeting/mod.rs`
- Modify: `services/agent-gateway/src/context.rs`
- Modify: `services/agent-gateway/src/lib.rs`

**Interfaces:**
- Consumes: `ContextStore::pool_clone() -> SqlitePool` added in this task.
- Produces: `MeetingStore::initialize`, `create`, `list`, `get_owned`, `record_chunk`, `missing_sequences`, `transition`, `replace_transcript`, `replace_artifact`, and `delete_owned`.

- [ ] **Step 1: Write failing store tests**

Add tests covering ownership, lifecycle transitions, chunk uniqueness, checksum conflict, transcript timing, meeting-local todos, and cascade deletion. Use this contract:

```rust
let meeting = store.create("user-a", "idem-1", 1_700_000_000.0).await?;
assert_eq!(meeting.state, MeetingState::Recording);
assert_eq!(store.create("user-a", "idem-1", 1_700_000_000.0).await?.id, meeting.id);
assert!(store.get_owned("user-b", &meeting.id).await?.is_none());
assert_eq!(store.record_chunk(&meeting.id, 0, 0, 15_000, "abc", 100).await?, ChunkWrite::Inserted);
assert_eq!(store.record_chunk(&meeting.id, 0, 0, 15_000, "abc", 100).await?, ChunkWrite::Existing);
assert_eq!(store.record_chunk(&meeting.id, 0, 0, 15_000, "other", 100).await?, ChunkWrite::Conflict);
```

- [ ] **Step 2: Run the focused tests and confirm failure**

Run on the server checkout:

```bash
cargo test -p ripple-agent-gateway meeting::store -- --nocapture
```

Expected: compilation fails because the meeting module and types do not exist.

- [ ] **Step 3: Implement schema, types, and store**

Create tables `meetings`, `meeting_chunks`, `meeting_transcript_segments`, `meeting_todos`, and `meeting_processing_jobs` with foreign keys and indexes. Define exact serialized states:

```rust
pub enum MeetingState { Recording, Paused, Uploading, Processing, Completed, Interrupted }
pub enum ProcessingStage { Upload, Transcript, Organization }
pub struct TranscriptSegment { pub id: i64, pub start_ms: i64, pub end_ms: i64, pub text: String, pub provisional: bool }
pub struct MeetingTodo { pub id: String, pub text: String, pub completed: bool, pub source_start_ms: Option<i64>, pub source_end_ms: Option<i64> }
```

Use transactions for state changes and replacement of transcript/artifact rows. Enforce `UNIQUE(user_id, idempotency_key)` and `PRIMARY KEY(meeting_id, sequence)`.

- [ ] **Step 4: Run store and migration tests**

```bash
cargo test -p ripple-agent-gateway meeting::store context::tests -- --nocapture
```

Expected: all tests pass and `PRAGMA foreign_key_check` returns no rows.

- [ ] **Step 5: Commit the server store slice**

```bash
git add services/agent-gateway/src/meeting services/agent-gateway/src/context.rs services/agent-gateway/src/lib.rs
git commit -m "feat(meeting): add durable recording data model"
```

### Task 2: Authenticated Meeting CRUD API

**Files:**
- Create: `services/agent-gateway/src/meeting_api.rs`
- Modify: `services/agent-gateway/src/main.rs`
- Test: `services/agent-gateway/src/main.rs` test module

**Interfaces:**
- Consumes: `MeetingStore` ownership-scoped methods from Task 1.
- Produces: create/list/detail/delete endpoints and `AppState.meetings: MeetingService`.
- Produces: ownership-scoped meeting-todo completion updates without touching the global todo table.

- [ ] **Step 1: Write failing authenticated API tests**

Test create idempotency, list isolation, detail isolation, delete cascade, invalid state input, and unauthenticated rejection. The create body is fixed:

```json
{"idempotency_key":"device-uuid","started_at":1700000000.0}
```

Expected create response: HTTP 201 with `{ "data": { "state": "recording" } }`; repeating it returns HTTP 200 with the same meeting ID.

- [ ] **Step 2: Run API tests and confirm 404/failure**

```bash
cargo test -p ripple-agent-gateway meeting_api -- --nocapture
```

- [ ] **Step 3: Implement handlers and routes**

Add:

```text
POST   /v1/meetings
GET    /v1/meetings
GET    /v1/meetings/{meeting_id}
DELETE /v1/meetings/{meeting_id}
PATCH  /v1/meetings/{meeting_id}/todos/{todo_id}
```

The todo patch body is `{ "completed": true | false }`. All handlers call the existing `authenticated_user`. Return 404 for foreign IDs so ownership is not disclosed. Keep meeting response objects separate from conversation/memory/todo types and prove that this route never writes the existing `todos` table.

- [ ] **Step 4: Run gateway API and auth tests**

```bash
cargo test -p ripple-agent-gateway meeting_api auth -- --nocapture
```

- [ ] **Step 5: Commit CRUD API**

```bash
git add services/agent-gateway/src/meeting_api.rs services/agent-gateway/src/main.rs
git commit -m "feat(meeting): expose authenticated meeting records"
```

### Task 3: Immutable Chunk Storage and Final Audio

**Files:**
- Create: `services/agent-gateway/src/meeting/storage.rs`
- Modify: `services/agent-gateway/src/meeting/mod.rs`
- Modify: `services/agent-gateway/src/meeting_api.rs`
- Modify: `services/agent-gateway/src/config.rs`
- Modify: `services/agent-gateway/src/main.rs`
- Modify: `deploy/agent-stack/start.sh`
- Modify: `deploy/agent-stack/status.sh`

**Interfaces:**
- Consumes: chunk metadata store from Task 1.
- Produces: `MeetingStorage::put_chunk`, `assemble_final_audio`, `decode_to_pcm16k`, `delete_meeting`; chunk upload/finalize/audio endpoints.

- [ ] **Step 1: Write failing storage and endpoint tests**

Cover maximum chunk size, SHA-256 verification, atomic temp-file rename, identical retry, checksum conflict, missing sequence report, path traversal rejection, and authenticated audio fetch.

- [ ] **Step 2: Run focused tests and confirm failure**

```bash
cargo test -p ripple-agent-gateway meeting::storage meeting_chunk_api -- --nocapture
```

- [ ] **Step 3: Implement storage and API contracts**

Add:

```text
PUT  /v1/meetings/{meeting_id}/chunks/{sequence}
POST /v1/meetings/{meeting_id}/finalize
GET  /v1/meetings/{meeting_id}/audio
```

The upload headers are `Content-Type: audio/mp4`, `X-Chunk-SHA256`, `X-Start-Ms`, and `X-End-Ms`. Stream each body to a same-directory temporary file, enforce `RIPPLE_MEETING_MAX_CHUNK_BYTES` (default 2 MiB), compute SHA-256 while writing, `sync_all`, then atomically rename to `<sequence>.m4a`.

Set `RIPPLE_FFMPEG_BIN` default to `ffmpeg`. Invoke it with `tokio::process::Command` and explicit argument arrays, never a shell string. `status.sh` must report FFmpeg availability without printing secrets.

Finalization accepts `{ "last_sequence": 17, "ended_at": 1700000300.0 }`. If sequences are missing, return HTTP 409 with `{ "missing_sequences": [3, 4] }`. Otherwise assemble `recording.m4a`, mark uploading complete, and schedule processing.

- [ ] **Step 4: Run storage, API, and full gateway tests**

```bash
cargo test -p ripple-agent-gateway meeting -- --nocapture
cargo test -p ripple-agent-gateway
```

- [ ] **Step 5: Commit chunk storage**

```bash
git add services/agent-gateway/src/meeting services/agent-gateway/src/meeting_api.rs services/agent-gateway/src/config.rs services/agent-gateway/src/main.rs deploy/agent-stack/start.sh deploy/agent-stack/status.sh
git commit -m "feat(meeting): store resumable audio chunks"
```

### Task 4: Provisional and Final Transcript Pipeline

**Files:**
- Create: `services/agent-gateway/src/meeting/processor.rs`
- Modify: `services/agent-gateway/src/meeting/mod.rs`
- Modify: `services/agent-gateway/src/adapters.rs`
- Test: `services/agent-gateway/src/meeting/processor.rs`

**Interfaces:**
- Consumes: `MeetingStorage::decode_to_pcm16k`, `ModelAdapters::transcribe`, and store replacement methods.
- Produces: `MeetingProcessor::transcribe_chunk`, `finalize_transcript`, and `reconcile_overlap`.

- [ ] **Step 1: Write failing transcript tests**

Use deterministic segments:

```rust
let merged = reconcile_overlap(
    &[(0, 15_000, "今天讨论发布计划"), (14_000, 30_000, "发布计划下周开始")],
);
assert_eq!(merged[0].text, "今天讨论发布计划");
assert_eq!(merged[1].text, "下周开始");
assert!(merged.windows(2).all(|pair| pair[0].end_ms <= pair[1].start_ms));
```

Also test silence, ASR retry, stale provisional replacement, and stable millisecond offsets.

- [ ] **Step 2: Confirm test failure**

```bash
cargo test -p ripple-agent-gateway meeting::processor::tests -- --nocapture
```

- [ ] **Step 3: Implement processing**

After each accepted chunk, spawn a bounded provisional ASR task. On finalization, decode the full timeline in bounded windows with one-second overlap, transcribe, reconcile duplicate boundary words, and replace provisional rows transactionally. Limit concurrent ASR work with a Tokio semaphore so a four-hour catch-up cannot starve realtime voice traffic.

- [ ] **Step 4: Run meeting and realtime regression tests**

```bash
cargo test -p ripple-agent-gateway meeting::processor endpointing orchestrator -- --nocapture
```

- [ ] **Step 5: Commit transcript pipeline**

```bash
git add services/agent-gateway/src/meeting services/agent-gateway/src/adapters.rs
git commit -m "feat(meeting): build time-aligned meeting transcripts"
```

### Task 5: Responses API Meeting Organization and Retry

**Files:**
- Modify: `services/agent-gateway/src/adapters.rs`
- Modify: `services/agent-gateway/src/meeting/processor.rs`
- Modify: `services/agent-gateway/src/meeting/mod.rs`
- Modify: `services/agent-gateway/src/meeting_api.rs`
- Test: `services/agent-gateway/src/meeting/processor.rs`

**Interfaces:**
- Consumes: final `TranscriptSegment` rows from Task 4.
- Produces: `MeetingArtifact { title, summary, todos }`, hierarchical organization, `POST /retry`.

- [ ] **Step 1: Write failing organization tests**

Assert a forced Responses API function call named `save_meeting_artifact` with schema fields `title`, `summary`, and `todos[]`. Reject plain text, malformed JSON, empty title, or more than 50 action items. Verify that mock output never inserts into the existing `todos` table.

- [ ] **Step 2: Confirm tests fail**

```bash
cargo test -p ripple-agent-gateway meeting_organization -- --nocapture
```

- [ ] **Step 3: Implement hierarchical Responses calls**

Group transcript sections by bounded character count while retaining `[start_ms-end_ms]` prefixes. Summarize each group with the Responses API, then make one final forced structured call. Parse into:

```rust
pub struct MeetingArtifact {
    pub title: String,
    pub summary: String,
    pub todos: Vec<MeetingTodoDraft>,
}
```

Set reasoning effort to `none`, use no external tools, and persist title/summary/todos in one transaction. Add `POST /v1/meetings/{meeting_id}/retry` with body `{ "stage": "transcript" | "organization" }`; completed stages are idempotent.

- [ ] **Step 4: Run all server tests**

```bash
cargo test -p ripple-agent-gateway
```

- [ ] **Step 5: Commit organization processing**

```bash
git add services/agent-gateway/src/adapters.rs services/agent-gateway/src/meeting services/agent-gateway/src/meeting_api.rs
git commit -m "feat(meeting): generate organized meeting artifacts"
```

### Task 6: TypeScript Meeting Client and Timeline Logic

**Files:**
- Create: `apps/mobile/src/meetings/types.ts`
- Create: `apps/mobile/src/meetings/api.ts`
- Create: `apps/mobile/src/meetings/timeline.ts`
- Create: `apps/mobile/tests/meeting-timeline.test.ts`
- Create: `apps/mobile/tests/meeting-api.test.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Consumes: server JSON contracts from Tasks 2-5.
- Produces: `Meeting`, `TranscriptSegment`, `meetingAtTime`, `createMeeting`, `meetings`, `meetingDetail`, `finalizeMeeting`, `retryMeeting`, `updateMeetingTodo`, `deleteMeeting`, and `meetingAudioBlob`.

- [ ] **Step 1: Write failing pure tests**

```ts
assert.equal(meetingAtTime(12_500, [
  { id: 1, start_ms: 0, end_ms: 10_000, text: 'a', provisional: false },
  { id: 2, start_ms: 10_000, end_ms: 20_000, text: 'b', provisional: false },
])?.id, 2)
```

Mock `fetch` to verify bearer auth, encoded IDs, 409 missing-sequence parsing, retry bodies, meeting-todo completion, deletion, and authenticated audio Blob loading.

- [ ] **Step 2: Confirm test failure**

```bash
cd apps/mobile
npx tsx --test tests/meeting-timeline.test.ts tests/meeting-api.test.ts
```

- [ ] **Step 3: Implement focused TypeScript modules**

Keep meeting API types out of `api.ts` so existing conversation/memory/todo code is unchanged. Define polling state from server `processing_stage` and `error` rather than inventing UI-only lifecycle values.

- [ ] **Step 4: Run new and existing TypeScript tests**

```bash
cd apps/mobile
npx tsx --test tests/meeting-timeline.test.ts tests/meeting-api.test.ts
npm run test:realtime
npm run test:mobile
```

- [ ] **Step 5: Commit client contracts**

```bash
git add apps/mobile/src/meetings apps/mobile/tests/meeting-timeline.test.ts apps/mobile/tests/meeting-api.test.ts apps/mobile/package.json apps/mobile/package-lock.json
git commit -m "feat(meeting): add mobile meeting data client"
```

### Task 7: Tauri Android Plugin and Foreground Service Lifecycle

**Files:**
- Create: `apps/mobile/src-tauri/src/meeting_recorder.rs`
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingRecorderPlugin.kt`
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingRecorderService.kt`
- Modify: `apps/mobile/src-tauri/src/lib.rs`
- Modify: `apps/mobile/src-tauri/Cargo.toml`
- Modify: `apps/mobile/src-tauri/capabilities/default.json`
- Modify: `apps/mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml`
- Modify: `apps/mobile/src-tauri/gen/android/app/build.gradle.kts`
- Test: `apps/mobile/src-tauri/gen/android/app/src/test/java/cn/minicpm/live/MeetingRecorderStateTest.kt`

**Interfaces:**
- Produces plugin commands `startMeeting`, `pauseMeeting`, `resumeMeeting`, `stopMeeting`, `getActiveMeeting`, `retryUploads`, and `deleteLocalMeeting`; event `meeting-state` with `meetingId`, `state`, `elapsedMs`, `level`, `network`, `lastUploadedSequence`, and `error`.

- [ ] **Step 1: Write failing package and Kotlin lifecycle tests**

Extend `mobile-package.test.mjs` to require the service declaration, `android:foregroundServiceType="microphone"`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MICROPHONE`, `POST_NOTIFICATIONS`, plugin registration, and no Apple-file references. Add a JVM state test that rejects `resume` from `stopped` and makes repeated `pause` idempotent.

- [ ] **Step 2: Confirm tests fail**

```bash
cd apps/mobile
npm run test:mobile
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
```

- [ ] **Step 3: Implement plugin and service state machine**

Register a Tauri 2 Android plugin class extending `app.tauri.plugin.Plugin`. Parse command args with `@InvokeArg`, run blocking work on `Dispatchers.IO`, and expose typed Rust wrappers through `PluginHandle::run_mobile_plugin`.

Declare the service as non-exported with `foregroundServiceType="microphone"`. Start it only in direct response to the visible Meeting screen action after runtime permission succeeds. Call `startForegroundService`, then `ServiceCompat.startForeground(..., FOREGROUND_SERVICE_TYPE_MICROPHONE)` immediately with an ongoing notification that returns to `MainActivity`.

- [ ] **Step 4: Run Rust, package, and Kotlin tests**

```bash
cd apps/mobile
cargo test --manifest-path src-tauri/Cargo.toml
npm run test:mobile
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
```

- [ ] **Step 5: Commit foreground lifecycle**

```bash
git add apps/mobile/src-tauri apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(meeting): add Android recording foreground service"
```

### Task 8: Encrypted Audio Capture, Manifest, and Resumable Upload

**Files:**
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingManifestStore.kt`
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingAudioCapture.kt`
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingCrypto.kt`
- Create: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingUploader.kt`
- Modify: `apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live/MeetingRecorderService.kt`
- Test: `apps/mobile/src-tauri/gen/android/app/src/test/java/cn/minicpm/live/MeetingManifestStoreTest.kt`
- Test: `apps/mobile/src-tauri/gen/android/app/src/test/java/cn/minicpm/live/MeetingUploaderTest.kt`

**Interfaces:**
- Consumes: Task 7 service lifecycle and Task 3 upload headers.
- Produces: immutable encrypted M4A chunks, recovery journal, upload cursor, offline retry.

- [ ] **Step 1: Write failing storage/upload tests**

Test deterministic filenames (`000000.m4a.enc`), atomic manifest replacement, AES-GCM roundtrip/tamper rejection, sequence cursor advancement only after acknowledgement, retry preserving order, checksum stability, and recovery of a journal without a finalized chunk.

- [ ] **Step 2: Confirm JVM tests fail**

```bash
cd apps/mobile
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
```

- [ ] **Step 3: Implement capture and upload**

Use `AudioRecord` at 16 kHz, mono, PCM 16-bit. Append frames to a per-chunk PCM journal and flush at most every second. At 10-15 seconds, finalize AAC-LC through `MediaCodec`/`MediaMuxer`, encrypt with AES-GCM using a non-exportable Android Keystore key, compute SHA-256 over the upload plaintext, then atomically mark the sequence ready. Derive offsets from accumulated recorded samples so pauses create no fake audio interval.

Upload one decrypted chunk at a time with the exact Task 3 headers. Use bounded exponential backoff with jitter (`1s, 2s, 4s ... 60s`), stop retries when offline, and continue from the durable acknowledgement cursor. Keep the access token in service memory only; after process restart, recording recovery works immediately and uploads resume after the foreground UI supplies a fresh token.

Check available storage before start and before every chunk rotation. Warn at 512 MiB free and finalize/stop safely at 256 MiB free. Treat `AudioRecord` failure or lost microphone availability as an explicit persisted interruption, finalize recoverable frames, and notify the user. `deleteLocalMeeting` may remove encrypted chunks only after the server delete succeeds or the user explicitly confirms deletion of a never-uploaded local draft.

- [ ] **Step 4: Run JVM tests and build Android debug**

```bash
cd apps/mobile
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
JAVA_HOME=/Applications/Android\ Studio.app/Contents/jbr npm run tauri -- android build --debug --target aarch64
```

- [ ] **Step 5: Commit durable recorder**

```bash
git add apps/mobile/src-tauri/gen/android/app/src/main/java/cn/minicpm/live apps/mobile/src-tauri/gen/android/app/src/test
git commit -m "feat(meeting): record encrypted resumable audio chunks"
```

### Task 9: Meeting Live Screen and Recovery UI

**Files:**
- Create: `apps/mobile/src/meetings/recorder.ts`
- Create: `apps/mobile/src/components/MeetingLive.tsx`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: Task 7 plugin commands/events and Task 6 meeting API.
- Produces: dedicated home entry, active recording screen, permission flow, live provisional transcript polling, and active-meeting recovery.

- [ ] **Step 1: Write failing package assertions**

Require a separate `meeting-live` screen and exact copy `会议模式 · 仅记录，不会回答`. Assert the Meeting start path never constructs `RealtimeSession`, never creates `LiveMedia`, and has only pause/resume/end controls. Assert offline copy says `录音正常，转写将在联网后补齐`.

- [ ] **Step 2: Confirm package tests fail**

```bash
cd apps/mobile
npm run test:mobile
```

- [ ] **Step 3: Implement UI and plugin wrapper**

Add `Screen` values `meetings`, `meeting-live`, and `meeting-detail`. Starting a meeting requests plugin microphone/notification permission, creates the server meeting with a device idempotency key, then starts the foreground service with meeting ID, server base URL, and access token. Subscribe to `meeting-state`, poll meeting details while visible, and call `getActiveMeeting` on authenticated App startup to restore the screen.

- [ ] **Step 4: Run frontend checks**

```bash
cd apps/mobile
npm run lint
npm run build
npm run test:mobile
npm run test:realtime
```

- [ ] **Step 5: Commit live meeting UI**

```bash
git add apps/mobile/src/meetings/recorder.ts apps/mobile/src/components/MeetingLive.tsx apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(meeting): add reliable live recording screen"
```

### Task 10: Meeting Center, Playback Timeline, and Local Action Items

**Files:**
- Create: `apps/mobile/src/components/MeetingCenter.tsx`
- Create: `apps/mobile/src/components/MeetingDetail.tsx`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/meeting-timeline.test.ts`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: Task 6 API/timeline functions and final audio Blob endpoint.
- Produces: independent meeting library, processing retry, synchronized player, meeting-local todo completion UI.

- [ ] **Step 1: Write failing UI/timeline assertions**

Test exact boundary lookup, no highlight in gaps, click-to-seek seconds conversion, object URL cleanup, processing-state labels, and that meeting todos render only inside `MeetingDetail`. Assert `App.tsx` does not pass them to existing `todos()` state or global Todo components.

- [ ] **Step 2: Confirm tests fail**

```bash
cd apps/mobile
npx tsx --test tests/meeting-timeline.test.ts
npm run test:mobile
```

- [ ] **Step 3: Implement center and detail**

Fetch audio with bearer authentication into a Blob URL, revoke it on detail unmount, and update `currentTime` at animation-frame cadence only while playing. `meetingAtTime(currentTime * 1000, segments)` drives one highlighted transcript row. Clicking a row sets `audio.currentTime = start_ms / 1000` and starts playback. Show independent retry controls for upload, transcript, and organization errors. Meeting-todo completion calls the meeting-scoped PATCH endpoint. After a successful server deletion, call `deleteLocalMeeting`; if local cleanup fails, show a retryable cache-cleanup warning without recreating the server record.

- [ ] **Step 4: Run all mobile tests and build**

```bash
cd apps/mobile
npm run lint
npm run build
npm run test:mobile
npm run test:realtime
npx tsx --test tests/meeting-timeline.test.ts tests/meeting-api.test.ts
```

- [ ] **Step 5: Commit Meeting Center**

```bash
git add apps/mobile/src/components/MeetingCenter.tsx apps/mobile/src/components/MeetingDetail.tsx apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests
git commit -m "feat(meeting): add synchronized meeting record center"
```

### Task 11: Server Smoke Contract and Remote Deployment

**Files:**
- Modify: `deploy/agent-stack/test-smoke-contract.py`
- Modify: `deploy/agent-stack/smoke-test.py`
- Modify: `deploy/agent-stack/README.md`

**Interfaces:**
- Consumes: completed server endpoints and processing jobs.
- Produces: authenticated real-audio smoke proof and documented runtime variables.

- [ ] **Step 1: Add failing smoke-contract tests**

Require meeting create, two idempotent chunk uploads, missing-sequence 409, finalize, polling to completion, authenticated audio retrieval, a non-empty final transcript/title/summary, meeting-local todos absent from `/v1/todos`, and deletion. Assert the script never prints the bearer token or transcript body.

- [ ] **Step 2: Run smoke contract locally on the server checkout**

```bash
python3 deploy/agent-stack/test-smoke-contract.py
```

Expected: failure until meeting smoke support is added.

- [ ] **Step 3: Implement the authenticated smoke path**

Use a generated short WAV fixture converted to M4A with configured FFmpeg. Upload through the real API and poll boundedly. Confirm a real Responses API organization request by requiring the completed artifact, not only a process health response.

- [ ] **Step 4: Deploy through the existing server workflow and verify**

On `140.143.229.103`, recheck branch, worktree, service `ExecStart`, data directory, FFmpeg, and current active jobs. Build `ripple-agent-gateway`, wait for a safe restart point, restart the actual service, then run:

```bash
cargo test -p ripple-agent-gateway
python3 deploy/agent-stack/test-smoke-contract.py
RIPPLE_SMOKE_ACCESS_TOKEN="<shell-provided-secret>" python3 deploy/agent-stack/smoke-test.py
```

The secret is supplied interactively or from the existing protected environment and must not be written into the plan, repository, shell history, or output.

- [ ] **Step 5: Commit deployment/smoke changes**

```bash
git add deploy/agent-stack/test-smoke-contract.py deploy/agent-stack/smoke-test.py deploy/agent-stack/README.md
git commit -m "test(meeting): verify live meeting processing"
```

### Task 12: Android Package and Real-Device Reliability Audit

**Files:**
- Modify: `apps/mobile/src-tauri/gen/android/app/src/androidTest/java/cn/minicpm/live/MeetingRecorderInstrumentedTest.kt`
- Modify: `apps/mobile/README.md`
- Modify: `docs/superpowers/specs/2026-08-06-meeting-mode-recording-design.md` only if verified behavior requires a factual correction.

**Interfaces:**
- Consumes: deployed server and complete Android feature.
- Produces: ABI-inspected APK, install/launch evidence, lifecycle evidence, four-hour recording artifact, and requirement audit.

- [ ] **Step 1: Add instrumentation assertions**

Cover foreground notification creation, start/pause/resume/stop, Activity background/return, persisted active meeting, notification tap return, and force-stop recovery metadata. Do not assert that Android force-stop can continue microphone access; assert that it preserves finalized data and marks the interruption.

- [ ] **Step 2: Run automated mobile verification**

```bash
cd apps/mobile
npm run lint
npm run build
npm run test:mobile
npm run test:realtime
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
JAVA_HOME=/Applications/Android\ Studio.app/Contents/jbr npm run tauri -- android build --debug --target aarch64
```

- [ ] **Step 3: Inspect, install, and launch the actual APK**

Inspect ZIP native ABIs, resolve the debug package/activity, install with `adb install -r`, launch it, and verify the active process/activity. Do not claim delivery from build output alone.

- [ ] **Step 4: Execute device reliability matrix**

Run and retain timestamped evidence for:

```text
30-minute normal recording
30-minute airplane-mode interval followed by automatic catch-up
lock screen and repeated App switching
UI task dismissal with foreground service continuing
microphone/audio-focus interruption
process termination and recoverable manifest
4-hour continuous recording with final playback
text click-to-seek and highlight drift <= 1 second
real title, summary, and meeting-local action items
zero Agent/TTS/tool events during recording
```

- [ ] **Step 5: Run final requirement audit and commit evidence docs**

Confirm every Global Constraint and design acceptance item against source, automated tests, server logs/status, authenticated requests, and the real device. Update `apps/mobile/README.md` with exact operational behavior and recovery limits, then commit:

```bash
git add apps/mobile/README.md apps/mobile/src-tauri/gen/android/app/src/androidTest
git commit -m "docs(meeting): document verified Android recording flow"
```

## Final Verification Commands

Server (`140.143.229.103`):

```bash
cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml
cargo test -p ripple-agent-gateway
python3 deploy/agent-stack/test-smoke-contract.py
```

Mobile (`/Users/lake/workspace/ripple-live/apps/mobile`):

```bash
npm run lint
npm run build
npm run test:mobile
npm run test:realtime
npx tsx --test tests/meeting-timeline.test.ts tests/meeting-api.test.ts
./src-tauri/gen/android/gradlew -p src-tauri/gen/android :app:testDebugUnitTest
JAVA_HOME=/Applications/Android\ Studio.app/Contents/jbr npm run tauri -- android build --debug --target aarch64
```

No implementation is complete until the authenticated server request and real Android recording/playback path both pass.
