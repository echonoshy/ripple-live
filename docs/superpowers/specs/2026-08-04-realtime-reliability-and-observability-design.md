# Ripple Live Realtime Reliability and Observability Design

## Summary

This iteration makes the existing realtime voice/video pipeline reliably diagnosable before attempting broader latency or architecture changes. It adds a dependency-aware readiness check, prevents silent wake-video stalls across client versions, gives every response one explicit terminal state, and records the first-token and first-audio milestones needed to optimize perceived latency.

The work deliberately leaves transport replacement, TLS termination, CORS tightening, memory-system consolidation, tool parallelism, and TTS model tuning for later iterations.

## Goals

1. Distinguish a live Gateway process from an Agent stack that can actually serve a conversation.
2. Make the wake-video frame handshake versioned, correlated, observable, and safe for one legacy compatibility window.
3. Ensure every created response ends as `response.done`, `response.cancelled`, or `response.failed`.
4. Measure transcript latency, Agent first-token latency, TTS first-PCM latency, and total turn latency with the same `response_id`.
5. Verify changes through automated protocol tests and an authenticated remote smoke test.

## Non-Goals

- Changing ASR, Agent, or TTS models and their GPU allocation.
- Reducing codec-frame accumulation or playback buffer sizes.
- Replacing JSON/Base64 WebSocket messages with binary PCM or WebRTC.
- Introducing WSS, a reverse proxy, or new authentication mechanisms.
- Parallelizing tools or changing tool-selection behavior.
- Consolidating `memories` and `memory_items`.
- Redesigning the mobile interface.

## Evidence and Root Causes

### Readiness false positive

The current `/health` handler reports Gateway configuration and tool count without contacting ASR, Agent, or TTS. On the observed host, Gateway, ASR, and TTS were running while the Agent service and `127.0.0.1:8712` were unavailable, but `/health` still returned `ok: true`.

Root cause: liveness and dependency readiness share one shallow endpoint.

### Wake-video turn never starts

The server correctly creates a pending activated turn and requires its `response_id` on `input.video.frame` and `input.video.commit`. Observed client events uploaded the frame and commit without that identifier, so the server rejected both as stale and later cancelled the pending turn as `superseded_before_frame`.

Root cause: the protocol has no negotiated version or client build identifier, and the server has no bounded compatibility rule for an unambiguous legacy frame commit.

### Failed responses remain ambiguous

`spawn_turn` currently emits a generic `error` when orchestration fails. The mobile client displays the error but does not receive a response terminal event that clears the current response, tool state, and audio state in one place.

Root cause: failures use a session-level error event instead of the response lifecycle contract.

### Latency cannot be attributed to user-visible milestones

The event store records total Agent generation and full TTS segment duration, but not first text delta or first PCM output. Mobile playback also logs locally without correlating playback start back to the server response.

Root cause: stage-completion metrics exist, but first-result milestones are missing.

## Considered Approaches

### A. Strict protocol cutover

Reject every client that does not send the new protocol version and correlated frame events.

This is the simplest server implementation, but it would immediately break already installed test builds and provide no migration evidence.

### B. Versioned protocol with bounded legacy compatibility — chosen

Advertise server protocol information, require new clients to identify their build and version, and accept a missing frame `response_id` only when exactly one pending wake-video turn exists. Record every compatibility use and never apply it when an identifier is present but wrong.

This fixes the observed failure while preserving strict correlation for current clients and provides evidence for removing compatibility later.

### C. Remove response correlation from frames

Always attach incoming frames to the latest pending turn.

This is rejected because delayed frames can be attached to a newer utterance after interruption or rapid follow-up speech.

## Chosen Design

### 1. Liveness and readiness

`GET /health` remains a cheap liveness endpoint. It returns HTTP 200 when the Gateway event loop is alive and keeps the current top-level `ok: true` compatibility shape.

Add `GET /ready`. It concurrently probes configured dependencies with short bounded requests:

- ASR: `GET` the configured ASR readiness URL, defaulting to `/health` on the `RIPPLE_ASR_URL` origin.
- Agent: `GET` the configured Agent readiness URL, defaulting to `/v1/models` on the `RIPPLE_AGENT_URL` origin and applying the configured bearer token.
- TTS: `GET` the configured TTS readiness URL, defaulting to `/health` on the `RIPPLE_TTS_URL` origin.
- Database: execute `SELECT 1` through the existing pool.

`RIPPLE_ASR_READINESS_URL`, `RIPPLE_AGENT_READINESS_URL`, and `RIPPLE_TTS_READINESS_URL` override those derived URLs for providers with different health routes. Each probe has a two-second timeout, independent of the normal model request timeout.

The response contains one structured entry per dependency with `ok`, `elapsed_ms`, and a stable error category. It must not expose API keys, authorization headers, full upstream bodies, or user data.

`/ready` returns HTTP 200 only when every required dependency succeeds; otherwise it returns HTTP 503. Mock backends are reported as ready without network access.

The deployment status and smoke scripts use `/ready` for release acceptance while preserving `/health` for process supervision.

### 2. Protocol identity and wake-video correlation

The protocol version for this iteration is integer `2`.

The mobile `session.start` event adds:

```json
{
  "type": "session.start",
  "protocol_version": 2,
  "client_build": "0.1.1",
  "mode": "video",
  "activation_mode": "wake"
}
```

Vite reads `apps/mobile/src-tauri/tauri.conf.json` at build time and defines the client build from its `version` field. Development sessions append `-dev`; packaged builds send the unmodified native package version.

`session.ready` echoes `protocol_version: 2` and includes `legacy_frame_correlation: true` while compatibility remains enabled. A client declaring a protocol newer than the server receives a session-level incompatibility error and the session does not start media capture.

Current clients continue to copy the server-provided `response_id` into both `input.video.frame` and `input.video.commit`.

For a client that omits `response_id`, the server may infer it only when:

1. the session declared no protocol version or a version lower than `2`;
2. exactly one `pending_turn` exists; and
3. no explicit mismatching identifier was supplied.

Each inference records `server.protocol.legacy_frame_correlated` with the inferred response ID and event kind. An explicitly wrong response ID remains stale and is never rewritten.

### 3. Response terminal states

Every emitted `response.created` must be followed by exactly one terminal event:

- `response.done` for success;
- `response.cancelled` for interruption or supersession;
- `response.failed` for orchestration or dependency failure.

`response.failed` contains `response_id`, a stable public `code`, and a user-safe `message`. Internal error details remain in server logs and `server.turn.failed`, not in the client payload.

The mobile client handles all three terminal events through one cleanup path that:

- finishes or clears audio as appropriate;
- clears `currentResponseId`, pending text, tool label, and interruption state;
- returns to `silent` in wake mode or `listening` in continuous mode; and
- calls `onError` only for `response.failed`.

Session/protocol errors that occur before `response.created` continue to use the top-level `error` event.

### 4. First-result observability

The Gateway records these events once per response or segment:

- `server.agent.first_delta`: elapsed milliseconds from Agent request start to the first non-empty text or tool-call delta.
- `server.tts.first_audio`: elapsed milliseconds from TTS segment start to the first non-empty PCM samples.
- Existing `server.transcript.completed`, `server.agent.completed`, `server.tts.completed`, and `server.turn.completed` remain unchanged.

The first Agent milestone records whether the first useful delta was `text` or `tool_call`, without storing its content. The TTS milestone records `segment_index` and sample count, without audio bytes.

The mobile client sends one `output.playback.started` event for each response when its AudioWorklet begins playback. The event includes `response_id` and locally buffered milliseconds. The server records it as `server.output.playback.started`, allowing speech-end-to-audible-response analysis across the whole chain.

Observability writes remain best effort: a database event-write failure must not fail the conversation.

## Error Handling

- Dependency probe timeout becomes a structured readiness failure, not a Gateway crash.
- A missing legacy frame identifier is inferred only under the bounded rules above.
- An explicit stale frame identifier is ignored and logged as it is today.
- A failed turn emits `response.failed` even if failure happens after partial text or audio; the client clears buffered output on failure.
- Cancellation wins over a late failure: once `response.cancelled` is emitted for an aborted task, that task must not emit another terminal event.
- Public error messages use stable categories such as `agent_unavailable`, `asr_failed`, `tts_failed`, and `internal_error`; raw upstream errors remain server-side.

## Testing

### Gateway unit and integration tests

- `/health` remains successful without reachable model dependencies.
- `/ready` returns 200 when all injected probes succeed and 503 with the failed dependency identified when one probe fails.
- A protocol-2 frame and commit with the correct response ID start the pending turn.
- A legacy event without a response ID is correlated when exactly one pending turn exists.
- An explicit wrong response ID is rejected and never inferred.
- An orchestration failure produces one `response.failed` terminal event.
- Agent and TTS first-result events are recorded at most once.

### Mobile tests

- `session.start` includes protocol and build identifiers.
- Requested video frames and commits preserve `response_id`.
- `response.failed` clears response, tool, text/audio, and state before surfacing the error.
- Playback-start is reported once with the active response ID.

### Remote acceptance

On `140.143.229.103`:

1. build and restart only the Gateway service after confirming the configured Agent dependency;
2. verify `/health` returns 200;
3. verify `/ready` returns 200 and all four dependencies report ready;
4. run an authenticated continuous text or audio turn through final text and TTS;
5. install the newly built local mobile package on the target simulator/device;
6. run wake-video activation and confirm one response ID across activation, frame, turn, Agent, TTS, playback, and terminal events;
7. interrupt one response and verify a single `response.cancelled` terminal event; and
8. make one controlled dependency unavailable only if a safe isolated test endpoint is available, verifying `/health` stays live while `/ready` becomes 503.

## Rollout and Compatibility

Deploy the Gateway readiness and compatibility behavior before distributing protocol-2 mobile builds. This order prevents old installed clients from silently stalling during the transition.

Track `server.protocol.legacy_frame_correlated`. Remove the fallback only after installed protocol-2 clients have been verified and the compatibility counter remains zero for seven consecutive days.

No database migration is required because all new observations use the existing events table.

## Success Criteria

- An unavailable Agent dependency makes `/ready` return HTTP 503 while `/health` remains HTTP 200.
- A protocol-2 wake-video turn cannot lose correlation between activation and frame commit.
- One legacy installed client can complete the same handshake through the bounded compatibility path, with a deprecation event recorded.
- Every created response has exactly one terminal event.
- A completed spoken response has correlated transcript, first Agent delta, first TTS audio, mobile playback start, and total-turn events.
- Automated Gateway and mobile tests pass, followed by remote service and device/simulator evidence.
