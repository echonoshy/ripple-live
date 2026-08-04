# Semantic Turn Endpointing and Stop Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent replies during an unfinished spoken sentence while preserving fast replies for complete speech, and consume spoken stop commands without generating TTS.

**Architecture:** The mobile client turns a VAD end into a tentative pause rather than an immediate commit.  The gateway evaluates a snapshot of the accumulated audio in a cancellable background task, returns a semantic endpoint decision keyed by `turn_id`, and preserves the audio plus resulting transcript until the client commits it.  Stop commands take the existing immediate barge-in cancellation path and are consumed before an Agent turn is created.

**Tech Stack:** TypeScript, React, AudioWorklet, `@ricky0123/vad-web`, Rust 2024, Axum WebSocket, Tokio, reqwest, OpenAI-compatible ASR/Agent APIs, Node test runner, Cargo tests.

## Global Constraints

- Increment `REALTIME_PROTOCOL_VERSION` from `3` to `4`; all tentative-turn messages carry a client-generated `turn_id`.
- VAD still detects candidate pauses at its existing 500 ms setting; `continue` and `uncertain` decisions use an exact 1,500 ms fallback timer.
- A new `input.speech_started` must clear local audio immediately and send `response.cancel` before normal input audio.
- Endpoint classification is tool-free, accepts only well-formed high-confidence `complete` results, and otherwise returns `uncertain`.
- Stop-command matching is normalized and whole-utterance only; it must not consume commands such as `停止计时`.
- A handled stop command clears buffers and creates neither a user nor an assistant conversation turn.
- Keep the gateway receive loop responsive while ASR/classification runs by using a cancellable Tokio task and result channel.

---

## File Structure

- Create `services/agent-gateway/src/endpointing.rs`: pure transcript normalization, exact stop-command matching, endpoint decision/result types, classifier JSON parsing, and tests.
- Modify `services/agent-gateway/src/lib.rs`: export the endpointing module.
- Modify `services/agent-gateway/src/adapters.rs`: add a bounded, tool-free endpoint-classifier request.
- Modify `services/agent-gateway/src/orchestrator.rs`: transcribe a snapshot, run deterministic/model endpoint evaluation, and return a reusable transcript.
- Modify `services/agent-gateway/src/protocol.rs`: expose the deserialized `turn_id` field.
- Modify `services/agent-gateway/src/main.rs`: own tentative evaluation state, select between WebSocket input and evaluator results, validate `turn_id`, preserve audio, and consume stop commands.
- Modify `apps/mobile/src/realtime/protocol.ts`: declare protocol v4 and provide turn-id creation.
- Modify `apps/mobile/src/realtime/RealtimeSession.ts`: implement the client endpointing state machine and handle decision/command events.
- Modify `apps/mobile/src/App.tsx`: call `speechPaused()` rather than immediate `commitInput()` on VAD end.
- Modify `apps/mobile/tests/realtime-session.test.ts`: cover client endpointing event order and the 1.5-second fallback.
- Modify `apps/mobile/tests/mobile-package.test.mjs`: assert protocol v4 and endpointing wiring.

### Task 1: Define and test gateway endpointing primitives

**Files:**
- Create: `services/agent-gateway/src/endpointing.rs`
- Modify: `services/agent-gateway/src/lib.rs`

**Interfaces:**
- Produces `pub enum EndpointDecision { Complete, Continue, Uncertain }` with `as_str()`.
- Produces `pub struct EndpointEvaluation { pub transcript: String, pub decision: EndpointDecision, pub reason: &'static str, pub classifier_latency_ms: Option<u128> }`.
- Produces `pub fn normalize_command(text: &str) -> String`, `pub fn is_stop_command(text: &str) -> bool`, `pub fn deterministic_decision(text: &str) -> Option<EndpointDecision>`, and `pub fn parse_classifier_decision(text: &str) -> Option<(EndpointDecision, f32)>`.
- Consumed later by `AgentOrchestrator::evaluate_turn_end` and `handle_socket`.

- [ ] **Step 1: Write failing unit tests for stop matching and classifier parsing**

```rust
#[test]
fn only_complete_stop_utterances_are_consumed() {
    assert!(is_stop_command("瑞波，停一下"));
    assert!(is_stop_command("你先不要说了"));
    assert!(!is_stop_command("停止计时"));
    assert!(!is_stop_command("不要说这个"));
}

#[test]
fn malformed_or_low_confidence_classifier_output_is_not_complete() {
    assert_eq!(parse_classifier_decision(r#"{\"decision\":\"complete\",\"confidence\":0.91}"#), Some((EndpointDecision::Complete, 0.91)));
    assert_eq!(parse_classifier_decision(r#"{\"decision\":\"complete\",\"confidence\":0.5}"#), Some((EndpointDecision::Complete, 0.5)));
    assert_eq!(parse_classifier_decision("answer now"), None);
}
```

- [ ] **Step 2: Run the targeted test and verify it fails because the module does not exist**

Run: `cargo test endpointing --manifest-path services/agent-gateway/Cargo.toml`

Expected: FAIL with an unresolved module or test target error.

- [ ] **Step 3: Implement the pure endpointing module and export it**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointDecision { Complete, Continue, Uncertain }

pub fn is_stop_command(text: &str) -> bool {
    let command = normalize_command(text);
    ["停", "停下", "停一下", "停止", "别说了", "不要说了", "不用说了", "先别说", "安静"]
        .iter()
        .any(|candidate| command == *candidate)
}
```

Strip only punctuation, whitespace, an optional leading Ripple wake name, and a small fixed list of polite fillers before comparing the entire remaining utterance.  Return `Continue` for trailing connective/lead-in phrases; return `Complete` only for deterministic complete question/command endings; leave all other text for classifier evaluation.  Add `pub mod endpointing;` to `lib.rs`.

- [ ] **Step 4: Run endpointing tests**

Run: `cargo test endpointing --manifest-path services/agent-gateway/Cargo.toml`

Expected: PASS.

- [ ] **Step 5: Commit the self-contained gateway primitives**

```bash
git add services/agent-gateway/src/endpointing.rs services/agent-gateway/src/lib.rs
git commit -m "feat(gateway): add voice endpointing primitives"
```

### Task 2: Add bounded semantic endpoint evaluation in the gateway

**Files:**
- Modify: `services/agent-gateway/src/adapters.rs`
- Modify: `services/agent-gateway/src/orchestrator.rs`
- Test: `services/agent-gateway/src/orchestrator.rs`

**Interfaces:**
- Consumes `EndpointDecision`, `EndpointEvaluation`, `deterministic_decision`, and `parse_classifier_decision` from Task 1.
- Produces `ModelAdapters::classify_turn_end(&self, transcript: &str) -> anyhow::Result<String>`.
- Produces `AgentOrchestrator::evaluate_turn_end(&self, audio: &[f32]) -> EndpointEvaluation`.
- `evaluate_turn_end` always returns an evaluation; ASR/model failures map to `Uncertain` with an explicit reason and an empty transcript only when ASR failed.

- [ ] **Step 1: Write failing evaluation tests using mock adapters**

```rust
#[tokio::test]
async fn endpoint_evaluation_marks_asr_failure_uncertain() {
    let evaluation = orchestrator_with_failing_asr().evaluate_turn_end(&[0.1; 1600]).await;
    assert_eq!(evaluation.decision, EndpointDecision::Uncertain);
    assert_eq!(evaluation.reason, "asr_error");
}

#[test]
fn deterministic_incomplete_phrase_skips_classifier() {
    assert_eq!(deterministic_decision("因为"), Some(EndpointDecision::Continue));
}
```

- [ ] **Step 2: Run the test and verify it fails because no evaluator exists**

Run: `cargo test endpoint_evaluation --manifest-path services/agent-gateway/Cargo.toml`

Expected: FAIL with `evaluate_turn_end` not found.

- [ ] **Step 3: Implement a tool-free, bounded classifier and evaluator**

```rust
pub async fn classify_turn_end(&self, transcript: &str) -> anyhow::Result<String> {
    self.complete_with_options(
        &[json!({"role":"system","content":"Return only JSON: {\\\"decision\\\":\\\"complete|continue\\\",\\\"confidence\\\":0..1}. Mark complete only when the Chinese utterance is clearly finished."}),
          json!({"role":"user","content":transcript})],
        &[], json!("none"), 0.0, 64,
    ).await.map(|reply| reply.content)
}
```

Factor the existing non-streaming completion request into a private option-bearing helper so normal Agent settings stay unchanged.  In `evaluate_turn_end`, transcribe once, first apply deterministic rules, then accept classifier `Complete` only when confidence is at least `0.75`; classifier errors, malformed JSON, low confidence, or no deterministic result become `Uncertain`.  Include classifier elapsed time only when a classifier call occurred.

- [ ] **Step 4: Run focused gateway tests**

Run: `cargo test endpointing --manifest-path services/agent-gateway/Cargo.toml && cargo test endpoint_evaluation --manifest-path services/agent-gateway/Cargo.toml`

Expected: PASS.

- [ ] **Step 5: Commit semantic evaluation**

```bash
git add services/agent-gateway/src/adapters.rs services/agent-gateway/src/orchestrator.rs
git commit -m "feat(gateway): evaluate tentative voice turn endings"
```

### Task 3: Make WebSocket endpoint evaluation cancellable and commit-safe

**Files:**
- Modify: `services/agent-gateway/src/protocol.rs`
- Modify: `services/agent-gateway/src/main.rs`
- Test: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Consumes `AgentOrchestrator::evaluate_turn_end` from Task 2 and `is_stop_command` from Task 1.
- `ClientEvent` gains `pub turn_id: Option<String>`.
- Produces wire events `input.turn.decision` and `input.command.handled`.
- Valid client event order is `input.speech_started(turn_id)` → zero or more `input.speech_resumed(turn_id)` / `input.turn.pause(turn_id)` → `input.commit(turn_id)`.

- [ ] **Step 1: Add failing WebSocket/state tests**

```rust
#[test]
fn late_evaluation_result_cannot_finalize_a_resumed_turn() {
    let mut state = EndpointState::speaking("turn-1");
    state.pause("turn-1");
    state.resume("turn-1");
    assert!(!state.accepts_result("turn-1"));
}

#[test]
fn stop_command_clears_audio_without_spawning_agent_turn() {
    let mut state = endpoint_state_with_audio("turn-2", vec![0.1; 1600]);
    assert!(state.consume_stop("turn-2", "不要说了"));
    assert!(state.audio.is_empty());
    assert!(state.transcript.is_none());
}
```

- [ ] **Step 2: Run the test and verify it fails because endpoint state is absent**

Run: `cargo test late_evaluation_result --manifest-path services/agent-gateway/Cargo.toml`

Expected: FAIL with `EndpointState` not found.

- [ ] **Step 3: Add endpoint task state and select it with WebSocket input**

```rust
let (endpoint_results, mut endpoint_results_rx) = mpsc::channel::<EndpointTaskResult>(4);

loop {
    tokio::select! {
        Some(result) = endpoint_results_rx.recv() => handle_endpoint_result(result, ...).await?,
        message = ws_receiver.next() => handle_client_message(message, ...).await?,
    }
}
```

Keep `active_turn_id`, a `pending_endpoint: Option<PendingEndpoint>`, and an optional reusable transcript.  On `input.turn.pause`, clone the current audio and spawn `evaluate_turn_end`; save its join handle.  On resumed/new speech, clear, commit, close, or a new pause, abort an obsolete handle and clear its transcript.  Ignore results whose `turn_id` does not equal the pending turn id.

On a valid result, record `server.input.endpoint_evaluated` with duration, transcript character count, decision, reason, and classifier latency.  If the transcript is an exact stop command, cancel any response, clear audio/frames/state, record `server.input.stop_command_handled`, and emit `input.command.handled`.  Otherwise store the transcript and emit `input.turn.decision`; do not create a response yet.

On `input.commit`, require the matching active `turn_id`, consume the buffered audio and reusable transcript, and pass that transcript as `spawn_turn`'s override.  If no valid evaluation completed, run an ordinary final transcribe path and apply the same stop matcher before spawning.  Continue to cancel an active response immediately on a new `input.speech_started`.

- [ ] **Step 4: Run gateway tests and static checks**

Run: `cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml && cargo test --manifest-path services/agent-gateway/Cargo.toml`

Expected: PASS.

- [ ] **Step 5: Commit protocol and server state machine**

```bash
git add services/agent-gateway/src/protocol.rs services/agent-gateway/src/main.rs
git commit -m "feat(gateway): gate voice commits on semantic endpointing"
```

### Task 4: Implement client protocol v4 and endpointing state machine

**Files:**
- Modify: `apps/mobile/src/realtime/protocol.ts`
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Test: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- `REALTIME_PROTOCOL_VERSION` is `4`.
- Produces `createTurnId(): string` with `crypto.randomUUID()` and a timestamp/random fallback.
- Replaces public `commitInput()` usage for VAD endpoints with `speechPaused(): void`.
- `RealtimeSession` emits `input.speech_started`, `input.speech_resumed`, `input.turn.pause`, and matching `input.commit` with `turn_id`.

- [ ] **Step 1: Add failing client endpointing tests with fake timers**

```ts
test('continue decision waits exactly 1.5 seconds before committing', async () => {
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted()
  session.speechPaused()
  receive({ type: 'input.turn.decision', turn_id: sent.at(-1)?.turn_id, decision: 'continue' })
  await advanceTimersByTimeAsync(1499)
  assert.equal(sent.some((event) => event.type === 'input.commit'), false)
  await advanceTimersByTimeAsync(1)
  assert.equal(sent.at(-1)?.type, 'input.commit')
})

test('speech resumption cancels a pending endpoint timer', async () => {
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted(); session.speechPaused()
  receive({ type: 'input.turn.decision', turn_id: sent.at(-1)?.turn_id, decision: 'uncertain' })
  await session.speechStarted()
  assert.equal(sent.at(-1)?.type, 'input.speech_resumed')
})
```

Use Node's built-in mocked timers rather than sleeping in tests.

- [ ] **Step 2: Run the endpointing tests and verify they fail**

Run: `npm run test:realtime -- --test-name-pattern="(continue decision|speech resumption)"`

Expected: FAIL because `speechPaused` and decision handling do not exist.

- [ ] **Step 3: Implement the state machine without reordering audio ahead of its pause**

```ts
private currentTurnId: string | null = null
private pendingTurnId: string | null = null
private endpointTimer: ReturnType<typeof setTimeout> | null = null

speechPaused() {
  if (!this.currentTurnId) return
  this.pendingTurnId = this.currentTurnId
  void this.sendEvent({ type: 'input.turn.pause', turn_id: this.currentTurnId })
}

private commitPendingTurn(turnId: string) {
  if (turnId !== this.pendingTurnId) return
  this.clearEndpointTimer()
  this.pendingTurnId = null
  this.currentTurnId = null
  void this.sendEvent({ type: 'input.commit', turn_id: turnId })
}
```

Send pause and commit at normal priority, because all prior audio appends are normal-priority sends and must reach the server first.  On `speechStarted`, if a pending turn exists, clear its timer and send `input.speech_resumed` for the same id; otherwise create a new id and retain the existing high-priority cancellation behavior.  For `complete`, commit immediately; for `continue` or `uncertain`, set a 1,500 ms timer.  Ignore all decision and handled-command events whose id does not match the pending/current turn.  Clear endpoint state in `discardInput`, `forceListen`, and `close`.

- [ ] **Step 4: Run all realtime session tests**

Run: `npm run test:realtime`

Expected: PASS.

- [ ] **Step 5: Commit client endpointing**

```bash
git add apps/mobile/src/realtime/protocol.ts apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/tests/realtime-session.test.ts
git commit -m "feat(mobile): delay voice commit until semantic endpoint"
```

### Task 5: Wire VAD pause behavior and run integration regressions

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`
- Modify: `apps/mobile/README.md`

**Interfaces:**
- Consumes `RealtimeSession.speechPaused()` from Task 4.
- The `LiveMedia.start` VAD-end callback invokes `session.speechPaused()`.
- Existing VAD-start callback continues to invoke `session.speechStarted()`.

- [ ] **Step 1: Add failing wiring assertions**

```js
test('mobile uses protocol v4 semantic endpointing', () => {
  assert.match(realtimeSource, /REALTIME_PROTOCOL_VERSION = 4/)
  assert.match(appSource, /void session\.speechPaused\(\)/)
  assert.doesNotMatch(appSource, /void session\.commitInput\(\)/)
  assert.match(realtimeSource, /setTimeout\([^)]*, 1_500\)/)
})
```

- [ ] **Step 2: Run the package test and verify it fails**

Run: `npm run test:mobile --prefix apps/mobile -- --test-name-pattern="semantic endpointing"`

Expected: FAIL because the VAD callback still commits directly.

- [ ] **Step 3: Make the minimal application and documentation changes**

```ts
}, () => {
  void session.speechPaused()
}, (level) => {
```

Update the mobile README's voice-turn description to state that VAD produces tentative pauses, complete semantic decisions commit immediately, and other pauses use a 1.5-second fallback.  Document that a spoken stop command silences output and is not added to chat history.

- [ ] **Step 4: Run complete local verification**

Run: `npm run test:mobile --prefix apps/mobile && npm run test:realtime --prefix apps/mobile && npm run build --prefix apps/mobile && cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml && cargo test --manifest-path services/agent-gateway/Cargo.toml`

Expected: all commands PASS.

- [ ] **Step 5: Commit application wiring and documentation**

```bash
git add apps/mobile/src/App.tsx apps/mobile/tests/mobile-package.test.mjs apps/mobile/README.md
git commit -m "feat: add semantic voice endpointing"
```

## Plan self-review

- **Spec coverage:** Tasks 1–3 implement exact stop matching, semantic evaluation, stale-result invalidation, gateway observability, transcript reuse, and non-blocking evaluation.  Tasks 4–5 implement protocol v4, the 1.5-second fallback, VAD wiring, and mobile regressions.
- **No-placeholders check:** The plan contains no `TODO`, `TBD`, or deferred implementation markers; every task has concrete files, interfaces, tests, verification commands, and commit boundaries.
- **Type consistency:** `turn_id` is the same JSON field in mobile protocol, `ClientEvent`, server state, endpoint results, decisions, and commits.  `EndpointDecision` is the shared server domain type, while its `as_str()` value is the protocol string.
