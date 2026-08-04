# Ripple Live Realtime Reliability and Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the realtime voice/video path dependency-aware, protocol-correlated, terminal-state-safe, and measurable from speech commit through audible playback.

**Architecture:** Keep the current React/Tauri client and Rust Gateway pipeline. Add a focused Rust readiness module, a versioned session handshake with a bounded legacy frame-correlation helper, one response-failure terminal event, and best-effort first-result events. Keep server implementation and deployment on `140.143.229.103:/home/lake/workspace/ripple-live`; keep mobile implementation, builds, and device/simulator checks on `/Users/lake/workspace/ripple-live`.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, Reqwest, SQLx/SQLite, React, TypeScript, Vite, Tauri 2, Node test runner with `tsx`.

## Global Constraints

- Protocol version is integer `2`.
- `/health` remains a shallow HTTP 200 liveness endpoint.
- `/ready` returns HTTP 200 only when ASR, Agent, TTS, and SQLite are ready; otherwise HTTP 503.
- Readiness probes time out after two seconds and never return credentials or upstream bodies.
- A missing frame `response_id` is inferred only for a pre-v2 client with exactly one pending turn; an explicit mismatch is never rewritten.
- Every emitted `response.created` ends exactly once as `response.done`, `response.cancelled`, or `response.failed`.
- Observability writes are best effort and contain no text delta, audio bytes, secrets, or user data.
- Do not change model selection, GPU allocation, TTS chunking, playback buffer sizes, tool routing, memory storage, CORS, or WebSocket transport.
- Use TDD for every behavior change: write the test, run it and observe the expected failure, implement the minimum, and rerun the focused plus surrounding tests.
- Preserve unrelated user changes in both worktrees.

---

## File Map

### Remote server worktree

- Create `services/agent-gateway/src/readiness.rs`: dependency probe types, URL construction, concurrent checks, and unit tests.
- Modify `services/agent-gateway/src/lib.rs`: export `readiness`.
- Modify `services/agent-gateway/src/config.rs`: readiness URLs and two-second timeout.
- Modify `services/agent-gateway/src/context.rs`: SQLite `SELECT 1` readiness probe and event-test query helper under `#[cfg(test)]`.
- Modify `services/agent-gateway/src/main.rs`: `/ready`, session protocol state, legacy frame correlation, `response.failed`, and playback-start ingestion.
- Modify `services/agent-gateway/src/orchestrator.rs`: stable stage markers and first Agent/TTS result events.
- Modify `deploy/agent-stack/status.sh`: report Gateway liveness and aggregate readiness separately.
- Modify `deploy/agent-stack/smoke-test.py`: authenticated protocol-v2 lifecycle smoke and terminal-state assertions.

### Local mobile worktree

- Create `apps/mobile/src/realtime/protocol.ts`: protocol constants and event payload builders shared by production code and tests.
- Create `apps/mobile/tests/realtime-session.test.ts`: behavioral protocol, failure cleanup, and playback-report tests.
- Create `apps/mobile/src/vite-env.d.ts`: declare the injected build constant.
- Modify `apps/mobile/src/realtime/RealtimeSession.ts`: use payload builders, handle `response.failed`, and report playback start once.
- Modify `apps/mobile/src/media/LiveMedia.ts`: surface AudioWorklet playback-start to the app.
- Modify `apps/mobile/src/App.tsx`: connect playback-start callback to the active realtime session.
- Modify `apps/mobile/vite.config.ts`: inject the native package version from `tauri.conf.json`.
- Modify `apps/mobile/package.json` and lockfile: add the focused `tsx` test command.

---

### Task 1: Dependency-aware Gateway readiness

**Files:**
- Create: `services/agent-gateway/src/readiness.rs`
- Modify: `services/agent-gateway/src/lib.rs`
- Modify: `services/agent-gateway/src/config.rs`
- Modify: `services/agent-gateway/src/context.rs`
- Modify: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Consumes: `Settings`, `ContextStore`, configured backend names and URLs.
- Produces: `pub async fn check(settings: &Settings, context: &ContextStore) -> ReadinessReport`, where `ReadinessReport` serializes `{ ok, dependencies }` and each dependency serializes `{ ok, elapsed_ms, error? }`.

- [ ] **Step 1: Write failing readiness tests**

Add tests to the new `readiness.rs` defining the desired API:

```rust
#[tokio::test]
async fn mock_backends_and_database_are_ready() {
    let directory = tempfile::tempdir().unwrap();
    let mut settings = Settings::from_env().unwrap();
    settings.data_dir = directory.path().join("data");
    settings.asr_backend = "mock".into();
    settings.agent_backend = "mock".into();
    settings.tts_backend = "mock".into();
    let context = ContextStore::open(&settings.database_path()).await.unwrap();

    let report = check(&settings, &context).await;

    assert!(report.ok);
    assert!(report.dependencies.values().all(|item| item.ok));
}

#[tokio::test]
async fn unreachable_required_dependency_makes_report_not_ready() {
    let directory = tempfile::tempdir().unwrap();
    let mut settings = Settings::from_env().unwrap();
    settings.data_dir = directory.path().join("data");
    settings.asr_backend = "mock".into();
    settings.agent_backend = "openai".into();
    settings.agent_readiness_url = "http://127.0.0.1:1/v1/models".into();
    settings.tts_backend = "mock".into();
    let context = ContextStore::open(&settings.database_path()).await.unwrap();

    let report = check(&settings, &context).await;

    assert!(!report.ok);
    assert_eq!(report.dependencies["agent"].error.as_deref(), Some("unreachable"));
}
```

Add an Axum route test in `main.rs` that calls `/health` and `/ready` using `test_state()` and asserts both are HTTP 200 with mock backends. Add a second state with an unreachable Agent readiness URL and assert `/health` is 200 while `/ready` is 503.

- [ ] **Step 2: Run the focused tests and verify RED**

Run remotely:

```bash
cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test readiness -- --nocapture
```

Expected: compilation fails because `readiness`, `agent_readiness_url`, and `/ready` do not exist.

- [ ] **Step 3: Implement readiness settings and probes**

Add to `Settings`:

```rust
pub asr_readiness_url: String,
pub agent_readiness_url: String,
pub tts_readiness_url: String,
pub readiness_timeout: Duration,
```

Derive defaults by parsing the configured model URL with `reqwest::Url`, replacing its path with `/health` for ASR/TTS and `/v1/models` for Agent. Allow exact overrides through `RIPPLE_ASR_READINESS_URL`, `RIPPLE_AGENT_READINESS_URL`, and `RIPPLE_TTS_READINESS_URL`. Set `readiness_timeout` to two seconds; do not add a public tuning variable in this iteration.

In `ContextStore`, add:

```rust
pub async fn readiness(&self) -> anyhow::Result<()> {
    sqlx::query("SELECT 1").execute(&self.pool).await?;
    Ok(())
}
```

In `readiness.rs`, define serializable report structs and run ASR, Agent, TTS, and database probes with `tokio::join!`. Treat `mock` as ready. For Agent only, apply `bearer_auth` with `agent_api_key`. Convert failures into only these public categories: `timeout`, `unreachable`, `http_status`, and `database`.

In `main.rs`, register `.route("/ready", get(ready))`. Return `(StatusCode::OK, Json(report))` when `report.ok`, otherwise `(StatusCode::SERVICE_UNAVAILABLE, Json(report))`.

- [ ] **Step 4: Run focused and full Gateway tests and verify GREEN**

```bash
cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test readiness -- --nocapture
cargo test
cargo fmt --check
```

Expected: all commands exit 0; the route test proves `/health` stays live when `/ready` is 503.

- [ ] **Step 5: Commit the readiness unit**

```bash
git add services/agent-gateway/src/readiness.rs services/agent-gateway/src/lib.rs services/agent-gateway/src/config.rs services/agent-gateway/src/context.rs services/agent-gateway/src/main.rs
git commit -m "fix(readiness): report unavailable model dependencies"
```

---

### Task 2: Versioned wake-video frame correlation

**Files:**
- Create: `apps/mobile/src/realtime/protocol.ts`
- Create: `apps/mobile/src/vite-env.d.ts`
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Modify: `apps/mobile/vite.config.ts`
- Modify: `apps/mobile/package.json`
- Modify: `apps/mobile/package-lock.json`
- Modify: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Produces mobile `createSessionStart(mode, activationMode)` and `createRequestedFrameEvents(responseId, frame, capturedAt)`.
- Produces server `FrameCorrelation::{Matched(String), Legacy(String), Stale}` from `correlate_pending_frame(protocol_version, pending_response_id, event_response_id)`.

- [ ] **Step 1: Add the mobile TypeScript test runner and failing protocol tests**

Add `tsx` as a dev dependency and this package script:

```json
"test:realtime": "tsx --test tests/realtime-session.test.ts"
```

Create `apps/mobile/tests/realtime-session.test.ts`:

```typescript
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  REALTIME_PROTOCOL_VERSION,
  createRequestedFrameEvents,
  createSessionStart,
} from '../src/realtime/protocol.ts'

test('session start declares protocol version and native build', () => {
  const event = createSessionStart('video', 'wake')
  assert.equal(event.protocol_version, 2)
  assert.equal(event.client_build.length > 0, true)
  assert.equal(REALTIME_PROTOCOL_VERSION, 2)
})

test('requested frame and commit preserve one response id', () => {
  assert.deepEqual(
    createRequestedFrameEvents('response-7', 'jpeg-data', 1234),
    [
      {
        type: 'input.video.frame',
        response_id: 'response-7',
        image: 'jpeg-data',
        mime_type: 'image/jpeg',
        captured_at: 1234,
      },
      { type: 'input.video.commit', response_id: 'response-7' },
    ],
  )
})
```

- [ ] **Step 2: Run the mobile protocol tests and verify RED**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
```

Expected: FAIL because `src/realtime/protocol.ts` does not exist.

- [ ] **Step 3: Implement mobile protocol payload builders**

In `vite.config.ts`, read `src-tauri/tauri.conf.json` with `readFileSync`, append `-dev` only when `mode === "development"`, and inject the JSON-encoded string as `__RIPPLE_CLIENT_BUILD__`.

Declare the constant in `src/vite-env.d.ts`:

```typescript
declare const __RIPPLE_CLIENT_BUILD__: string
```

Create `protocol.ts` with:

```typescript
export const REALTIME_PROTOCOL_VERSION = 2

const clientBuild =
  typeof __RIPPLE_CLIENT_BUILD__ === 'string'
    ? __RIPPLE_CLIENT_BUILD__
    : '0.1.1-test'

export function createSessionStart(mode: RealtimeMode, activationMode: ActivationMode) {
  return {
    type: 'session.start',
    protocol_version: REALTIME_PROTOCOL_VERSION,
    client_build: clientBuild,
    mode,
    activation_mode: activationMode,
  }
}
```

Implement `createRequestedFrameEvents` exactly as asserted. Move `RealtimeMode` and `ActivationMode` to `protocol.ts` and re-export them from `RealtimeSession.ts` to preserve existing imports. Replace the handwritten session-start and frame-request payloads in `RealtimeSession` with these builders.

- [ ] **Step 4: Add failing server correlation tests**

Replace the existing `pending_video_turn_only_accepts_its_own_response_id` test with:

```rust
#[test]
fn protocol_two_requires_exact_frame_response_id() {
    assert_eq!(
        correlate_pending_frame(Some(2), "current", Some("current")),
        FrameCorrelation::Matched("current".into())
    );
    assert_eq!(
        correlate_pending_frame(Some(2), "current", None),
        FrameCorrelation::Stale
    );
}

#[test]
fn legacy_client_can_infer_only_a_missing_identifier() {
    assert_eq!(
        correlate_pending_frame(None, "current", None),
        FrameCorrelation::Legacy("current".into())
    );
    assert_eq!(
        correlate_pending_frame(None, "current", Some("stale")),
        FrameCorrelation::Stale
    );
}
```

- [ ] **Step 5: Run the server correlation tests and verify RED**

```bash
cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test pending_frame -- --nocapture
```

Expected: compilation fails because `FrameCorrelation` and `correlate_pending_frame` do not exist.

- [ ] **Step 6: Implement server protocol negotiation and compatibility**

Add `const REALTIME_PROTOCOL_VERSION: u32 = 2`, session fields `client_protocol_version: Option<u32>` and `client_build: Option<String>`, and parse them on `session.start`.

If a client declares a version greater than 2, emit:

```json
{
  "type": "error",
  "code": "unsupported_protocol",
  "message": "客户端协议版本高于当前服务端，请升级服务端"
}
```

Do not emit `session.ready` for that start event. Otherwise include `protocol_version: 2` and `legacy_frame_correlation: true` in `session.ready`.

Use `correlate_pending_frame` in both `input.video.frame` and `input.video.commit`. On `Legacy`, continue with the inferred pending response ID and record `server.protocol.legacy_frame_correlated` with `response_id`, `event_kind`, and `client_build`. On `Stale`, preserve the current ignore behavior.

- [ ] **Step 7: Verify protocol GREEN on both worktrees**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run lint
npm run build

cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test pending_frame -- --nocapture
cargo test
cargo fmt --check
```

Expected: all commands exit 0; mobile events carry response IDs and server tests distinguish missing legacy IDs from explicit stale IDs.

- [ ] **Step 8: Commit protocol changes in their owning worktrees**

Remote server commit:

```bash
git add services/agent-gateway/src/main.rs
git commit -m "fix(realtime): correlate wake video frames"
```

Local mobile commit:

```bash
git add apps/mobile/src/realtime/protocol.ts apps/mobile/src/vite-env.d.ts apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/vite.config.ts apps/mobile/package.json apps/mobile/package-lock.json apps/mobile/tests/realtime-session.test.ts
git commit -m "fix(realtime): declare correlated mobile protocol"
```

---

### Task 3: Single terminal state for failed responses

**Files:**
- Modify: `services/agent-gateway/src/main.rs`
- Modify: `services/agent-gateway/src/orchestrator.rs`
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Modify: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- Produces server `response.failed` with `{ response_id, code, message }`.
- Produces mobile `finishResponse(kind: 'done' | 'cancelled' | 'failed', message?: string)` cleanup path.

- [ ] **Step 1: Write a failing mobile failure-cleanup test**

Extend `realtime-session.test.ts` with a fake options object that records state, tool label, assistant text, audio completion, interruption, and errors. Construct `RealtimeSession`, call its event handler through a narrow test cast, send `response.created`, `response.tool.started`, then `response.failed`, and assert:

```typescript
assert.equal(states.at(-1), 'listening')
assert.equal(tools.at(-1), '')
assert.equal(assistantTexts.at(-1), '')
assert.equal(audioClears, 1)
assert.deepEqual(errors, ['Agent 服务暂时不可用'])
```

Use continuous mode in this test; add a second assertion that wake mode returns to `silent`.

- [ ] **Step 2: Run the mobile test and verify RED**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
```

Expected: FAIL because `response.failed` is not handled and current cleanup is duplicated.

- [ ] **Step 3: Implement one client terminal cleanup path**

Add `code?: string` to `RealtimeEvent`. Extract a private `finishResponse` method. For success call `onAudioDone`; for cancellation and failure call `onInterrupted` so `App.tsx` clears playback. All outcomes clear `currentResponseId`, `assistantText`, `outputActive`, `interruptPending`, the tool label, and return to the correct idle state. Failure then calls `onError` with the public message.

Route `response.done`, `response.cancelled`, and `response.failed` through that method. Keep the top-level `error` case only for errors outside an active response.

- [ ] **Step 4: Write a failing Gateway terminal-event test**

Extract a small `failed_response_event(response_id, error)` helper and test:

```rust
#[test]
fn failed_response_is_public_and_correlated() {
    let event = failed_response_event(
        "response-9",
        &anyhow::anyhow!("AGENT_FAILED: upstream included a private body"),
    );
    assert_eq!(event["type"], "response.failed");
    assert_eq!(event["response_id"], "response-9");
    assert_eq!(event["code"], "agent_unavailable");
    assert_eq!(event["message"], "Agent 服务暂时不可用");
    assert!(!event.to_string().contains("private body"));
}
```

- [ ] **Step 5: Run the Gateway terminal test and verify RED**

```bash
cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test failed_response -- --nocapture
```

Expected: compilation fails because `failed_response_event` does not exist.

- [ ] **Step 6: Implement stage markers and response failure event**

Add stable internal contexts at orchestration boundaries:

- ASR request: `ASR_FAILED`
- Agent stream creation or consumption: `AGENT_FAILED`
- TTS stream creation or consumption: `TTS_FAILED`

Map those contexts to public codes/messages in `failed_response_event`; all other errors map to `internal_error` and `本次处理失败，请重试`. Keep the full `error` chain in logs and `server.turn.failed`.

In `spawn_turn`, replace the generic response-level `error` event with the mapped `response.failed` event. Do not change pre-response session errors. Aborted Tokio tasks continue to terminate through `cancel_response`, so they cannot emit a late failure.

- [ ] **Step 7: Verify terminal-state GREEN**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run lint

cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test failed_response -- --nocapture
cargo test
cargo fmt --check
```

Expected: all commands exit 0 and raw upstream details never enter the client event.

- [ ] **Step 8: Commit terminal lifecycle changes**

Remote:

```bash
git add services/agent-gateway/src/main.rs services/agent-gateway/src/orchestrator.rs
git commit -m "fix(realtime): terminate failed responses cleanly"
```

Local:

```bash
git add apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/tests/realtime-session.test.ts
git commit -m "fix(realtime): clear failed mobile responses"
```

---

### Task 4: First-delta, first-audio, and playback milestones

**Files:**
- Modify: `services/agent-gateway/src/orchestrator.rs`
- Modify: `services/agent-gateway/src/main.rs`
- Modify: `services/agent-gateway/src/context.rs`
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Modify: `apps/mobile/src/media/LiveMedia.ts`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- Produces events `server.agent.first_delta`, `server.tts.first_audio`, and `server.output.playback.started`.
- Produces `RealtimeSession.outputPlaybackStarted(bufferedMs: number): void`.
- Adds `LiveMediaOptions.onPlaybackStarted(bufferedMs: number): void`.

- [ ] **Step 1: Write failing Gateway milestone unit tests**

Add a pure helper test for Agent delta classification:

```rust
#[test]
fn first_useful_delta_distinguishes_text_and_tool_calls() {
    assert_eq!(useful_delta_kind(&json!({"choices":[{"delta":{"content":"你"}}]})), Some("text"));
    assert_eq!(useful_delta_kind(&json!({"choices":[{"delta":{"tool_calls":[{"index":0}]}}]})), Some("tool_call"));
    assert_eq!(useful_delta_kind(&json!({"choices":[{"delta":{}}]})), None);
}
```

Add an orchestrator mock-turn test that queries the events table and asserts exactly one `server.agent.first_delta` and at least one `server.tts.first_audio` for its response ID. Add a `#[cfg(test)]` `ContextStore::events_for_response(response_id)` helper that filters JSON payloads with SQLite `json_extract`.

- [ ] **Step 2: Run milestone tests and verify RED**

```bash
cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test first_useful_delta -- --nocapture
cargo test records_first_result_milestones -- --nocapture
```

Expected: compilation fails because the classifier and milestone events do not exist.

- [ ] **Step 3: Implement Gateway first-result milestones**

Keep `agent_first_delta_recorded` outside the tool-round loop so only the first useful delta for the whole response is recorded. On the first classified chunk, record elapsed time from that round's `agent_started` and `kind`, without content.

In `stream_speech`, keep `first_audio_recorded` per segment. On the first non-empty PCM output, record elapsed time from `segment_started`, `segment_index`, and the output sample count before buffering or emitting audio.

Both calls use the existing best-effort `record_flow_event`; event-write failure must not interrupt generation or speech.

- [ ] **Step 4: Write a failing mobile playback-report test**

In `realtime-session.test.ts`, install a fake transport through the test cast, handle `response.created` for `response-11`, call `outputPlaybackStarted(450)` twice, and assert the transport receives exactly one event:

```typescript
assert.deepEqual(sent, [{
  type: 'output.playback.started',
  response_id: 'response-11',
  buffered_ms: 450,
}])
```

- [ ] **Step 5: Run the playback test and verify RED**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
```

Expected: FAIL because `outputPlaybackStarted` does not exist.

- [ ] **Step 6: Implement mobile and server playback correlation**

Add `onPlaybackStarted` to `LiveMediaOptions` and call it from the AudioWorklet `playback-started` message with `bufferedMs ?? 0`.

In `RealtimeSession`, reset a `playbackStartedReported` boolean on `response.created` and terminal cleanup. `outputPlaybackStarted` sends the event only when a current response exists and the flag is false.

In `App.tsx`, declare `let session: RealtimeSession` immediately before constructing `LiveMedia`, pass `onPlaybackStarted: bufferedMs => session.outputPlaybackStarted(bufferedMs)` in the media options, and then assign `session = new RealtimeSession(...)`. Playback cannot begin before `session.connect()` and media startup, so the closure is assigned before it can run and no global state is introduced.

In `main.rs`, accept `output.playback.started`, require its `response_id` to match the current active response, clamp `buffered_ms` to `0..=10_000`, and best-effort record `server.output.playback.started`. Ignore stale playback events.

- [ ] **Step 7: Verify milestone GREEN**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run test:mobile
npm run lint
npm run build

cd /home/lake/workspace/ripple-live/services/agent-gateway
cargo test records_first_result_milestones -- --nocapture
cargo test
cargo fmt --check
```

Expected: all commands exit 0; each response has one first Agent delta, each TTS segment has one first-audio event, and mobile reports playback once.

- [ ] **Step 8: Commit observability changes**

Remote:

```bash
git add services/agent-gateway/src/orchestrator.rs services/agent-gateway/src/main.rs services/agent-gateway/src/context.rs
git commit -m "feat(observability): record first response milestones"
```

Local:

```bash
git add apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/src/media/LiveMedia.ts apps/mobile/src/App.tsx apps/mobile/tests/realtime-session.test.ts
git commit -m "feat(observability): report mobile playback start"
```

---

### Task 5: Deployment scripts and authenticated lifecycle smoke

**Files:**
- Modify: `deploy/agent-stack/status.sh`
- Modify: `deploy/agent-stack/smoke-test.py`

**Interfaces:**
- Consumes: `RIPPLE_SMOKE_ACCESS_TOKEN` and optional `RIPPLE_SMOKE_SERVER`, defaulting to `127.0.0.1:8700`.
- Produces: smoke output that names readiness, protocol version, tool loop, terminal uniqueness, interruption recovery, first text, first audio, and playback event persistence.

- [ ] **Step 1: Write failing script contract tests**

Add dependency-free source-contract assertions to a new `deploy/agent-stack/test-smoke-contract.py` using `unittest`. Assert that:

- the smoke script reads `RIPPLE_SMOKE_ACCESS_TOKEN` and never prints it;
- the WebSocket URL includes `access_token`;
- `session.start` declares protocol version 2;
- receive loops treat `response.failed` as terminal failure;
- the terminal-event set is exactly `done`, `cancelled`, and `failed`; and
- `status.sh` queries both `/health` and `/ready`.

- [ ] **Step 2: Run script contract tests and verify RED**

```bash
cd /home/lake/workspace/ripple-live
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
```

Expected: FAIL because the current smoke is unauthenticated and status has no readiness check.

- [ ] **Step 3: Implement status and smoke lifecycle checks**

Update `status.sh` so hardcoded component health remains diagnostic, then add separate lines:

```text
gateway liveness: ok|unavailable
gateway readiness: ok|unavailable
```

Update `smoke-test.py` to require `RIPPLE_SMOKE_ACCESS_TOKEN`, URL-encode it without printing it, and send protocol-v2 `session.start`. Centralize receive-loop terminal handling in one helper that rejects `response.failed`, rejects a second terminal event for the same response, and returns the final event.

After first received audio, send `output.playback.started` with the active response ID and measured buffered value `450`. Query the local SQLite events database read-only and assert the response has `server.agent.first_delta`, `server.tts.first_audio`, and `server.output.playback.started` before printing timings.

- [ ] **Step 4: Run contract and syntax tests and verify GREEN**

```bash
cd /home/lake/workspace/ripple-live
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
python3 -m py_compile deploy/agent-stack/smoke-test.py deploy/agent-stack/test-smoke-contract.py
bash -n deploy/agent-stack/status.sh
```

Expected: all commands exit 0.

- [ ] **Step 5: Commit deployment verification changes**

```bash
git add deploy/agent-stack/status.sh deploy/agent-stack/smoke-test.py deploy/agent-stack/test-smoke-contract.py
git commit -m "test(smoke): verify realtime terminal lifecycle"
```

---

### Task 6: Full verification, remote rollout, and mobile evidence

**Files:**
- Verify: all files changed in Tasks 1–5.
- Update only if results require factual corrections: `README.md` and the confirmed design spec.

**Interfaces:**
- Consumes: built Gateway binary, active authenticated account token, local mobile toolchain, available simulator/device.
- Produces: fresh test output, remote listener/readiness evidence, correlated event evidence, and installed mobile launch evidence.

- [ ] **Step 1: Run the complete remote server verification suite**

```bash
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live/services/agent-gateway && cargo fmt --check && cargo test && cargo build --release'
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live && python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v && python3 -m py_compile deploy/agent-stack/smoke-test.py && bash -n deploy/agent-stack/status.sh'
```

Expected: every command exits 0 before touching the running service.

- [ ] **Step 2: Confirm the actual Agent dependency before restart**

Read the current Gateway unit environment without printing secrets. Resolve `RIPPLE_AGENT_URL` or its default, then request its configured readiness endpoint with the appropriate bearer token while redacting the token from output.

Expected: the Agent dependency is reachable. If it is not, stop rollout, leave the current Gateway process untouched, and report the exact missing listener/configuration.

- [ ] **Step 3: Restart only the Gateway and verify liveness/readiness**

Build already completed in Step 1. Restart only `ripple-agent-gateway.service`, then run:

```bash
curl --fail --silent http://127.0.0.1:8700/health
curl --fail --silent http://127.0.0.1:8700/ready
./deploy/agent-stack/status.sh
```

Expected: `/health` and `/ready` both return success, and readiness names ASR, Agent, TTS, and database as ready.

- [ ] **Step 4: Run the authenticated remote smoke**

Provide the existing test account token only through `RIPPLE_SMOKE_ACCESS_TOKEN` and run `deploy/agent-stack/smoke-test.py` without echoing the variable.

Expected: protocol 2, tool loop, interruption, resumed response, first text, first audio, playback persistence, and exactly one terminal event per response all pass.

- [ ] **Step 5: Run the complete local mobile verification suite**

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
```

Expected: every command exits 0 with no warnings treated as errors.

- [ ] **Step 6: Build and install the target mobile package**

For iOS simulator/device, build from the local Mac using the existing Tauri/Xcode workflow and install on the currently available iPhone 17 Pro simulator or paired device. For Android, use explicit JDK 17 and `--target aarch64`, inspect the APK ZIP for only `lib/arm64-v8a`, then install with `adb install -r` only when the target device is connected.

Do not claim installation if no target is available; report build evidence and the missing device boundary separately.

- [ ] **Step 7: Verify the original wake-video failure on the installed build**

Run one wake-video request. Query the remote events database by its response ID and verify this order:

```text
server.activation.accepted
client.input.video.frame
client.input.video.commit
server.turn.started
server.agent.first_delta
server.tts.first_audio
server.output.playback.started
server.turn.completed
```

For the protocol-v2 build, assert there is no `server.protocol.legacy_frame_correlated` event. Then exercise one known legacy build only if it remains safely installable and assert the bounded compatibility event appears.

- [ ] **Step 8: Re-read the spec and record any unmet acceptance item**

Compare every Success Criteria bullet in `docs/superpowers/specs/2026-08-04-realtime-reliability-and-observability-design.md` with fresh command or event evidence. Report any item not proven as incomplete instead of inferring success from build output.

---

## Execution Notes

- Tasks 1, 2 server half, 3 server half, 4 server half, 5, and remote portions of 6 run on `140.143.229.103` as required by `AGENTS.md`.
- Tasks 2 mobile half, 3 mobile half, 4 mobile half, and mobile portions of 6 run on the local Mac.
- Never copy `.env`, access tokens, SQLite databases, assets, or model state between the remote and local worktrees.
- Before every commit, inspect only the intended paths and preserve unrelated changes.
- Before claiming completion, invoke `superpowers:verification-before-completion` and rerun the full commands from Task 6.
