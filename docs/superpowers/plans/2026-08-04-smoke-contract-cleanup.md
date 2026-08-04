# Smoke Contract Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the deployment smoke test validate the authenticated semantic-voice and on-demand-video protocol, then remove one obsolete source backup.

**Architecture:** Keep the integration test as one Python entrypoint. Extract small event-construction and event-waiting helpers so a standard-library contract test can validate protocol correlation without loading models. The full smoke uses a supplied access token, stable generated turn IDs, and an embedded valid JPEG only after the gateway requests it.

**Tech Stack:** Python 3, `unittest`, `httpx`, `numpy`, `websockets`, Rust gateway event store.

## Global Constraints

- Use the Responses API endpoint only.
- Do not print, store, or commit `RIPPLE_SMOKE_ACCESS_TOKEN`.
- Keep server changes and deployment work on `140.143.229.103`; Android remains local-only.
- Do not touch the user’s uncommitted `context.rs` or `memory.rs` changes in the main worktree.

---

### Task 1: Add protocol-contract tests

**Files:**
- Create: `deploy/agent-stack/test-smoke-contract.py`
- Test: `deploy/agent-stack/test-smoke-contract.py`

**Interfaces:**
- Consumes: `build_realtime_url(server, access_token)`, `voice_turn_events(turn_id, audio)`, `requested_frame_events(response_id)`, and `check_terminal_event(...)` from `smoke-test.py`.
- Produces: a hermetic contract suite that fails if token encoding, correlated `turn_id`, JPEG frame metadata, or terminal-state validation regresses.

- [ ] **Step 1: Write the failing tests**

```python
def test_voice_turn_events_reuse_one_turn_id(self):
    events = module.voice_turn_events("turn-7", b"\x00\x00\x00\x00")
    self.assertEqual([event["turn_id"] for event in events if "turn_id" in event], ["turn-7", "turn-7"])

def test_requested_frame_is_jpeg_and_correlated(self):
    frame, commit = module.requested_frame_events("response-9")
    self.assertEqual(frame["response_id"], "response-9")
    self.assertEqual(frame["mime_type"], "image/jpeg")
    self.assertEqual(commit["response_id"], "response-9")
```

- [ ] **Step 2: Run the contract suite and verify it fails because the helpers do not exist**

Run: `python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v`

Expected: FAIL with missing helper attributes.

- [ ] **Step 3: Add the helper implementations to `smoke-test.py`**

```python
def voice_turn_events(turn_id: str, audio: bytes) -> list[dict]:
    return [
        {"type": "input.speech_started", "turn_id": turn_id},
        {"type": "input.audio.append", "audio": base64.b64encode(audio).decode("ascii"), "sample_rate": 16_000},
        {"type": "input.commit", "turn_id": turn_id},
    ]
```

- [ ] **Step 4: Run the contract suite and verify it passes**

Run: `python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v`

Expected: PASS.

### Task 2: Upgrade the authenticated realtime smoke

**Files:**
- Modify: `deploy/agent-stack/smoke-test.py`
- Test: `deploy/agent-stack/test-smoke-contract.py`

**Interfaces:**
- Consumes: gateway protocol v3, `RIPPLE_SMOKE_ACCESS_TOKEN`, and the helper functions from Task 1.
- Produces: a full smoke that validates semantic Gate ignore, server-requested JPEG upload, response completion, audio playback milestone, cancellation, and recovery.

- [ ] **Step 1: Write a failing contract assertion for the frame payload and response ID**

```python
frame, commit = module.requested_frame_events("response-9")
self.assertTrue(base64.b64decode(frame["image"]).startswith(b"\xff\xd8"))
self.assertEqual(commit, {"type": "input.video.commit", "response_id": "response-9"})
```

- [ ] **Step 2: Run the contract suite and verify the assertion fails**

Run: `python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v`

Expected: FAIL until the fixture and helper are implemented.

- [ ] **Step 3: Implement server-requested frame handling**

Use a constant valid JPEG fixture in `smoke-test.py`. Send it only after receiving `input.frame.requested`; pass the exact response ID into both `input.video.frame` and `input.video.commit`. Use a separate TTS-created explicit visual question and correlated `turn_id` to reach that request. Fail on error, mismatched identifier, terminal failure, or timeout.

- [ ] **Step 4: Run static and contract verification**

Run: `python3 -m py_compile deploy/agent-stack/smoke-test.py deploy/agent-stack/test-smoke-contract.py && python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v`

Expected: PASS.

### Task 3: Remove the obsolete backup and deploy verification

**Files:**
- Delete: `services/agent-gateway/src/context.rs.orig`
- Modify remotely: `/home/lake/workspace/ripple-live/deploy/agent-stack/smoke-test.py`
- Create remotely: `/home/lake/workspace/ripple-live/deploy/agent-stack/test-smoke-contract.py`
- Delete remotely: `/home/lake/workspace/ripple-live/services/agent-gateway/src/context.rs.orig`

**Interfaces:**
- Consumes: the verified smoke files from Tasks 1–2.
- Produces: a remote checkout with the current smoke contract and no historical source backup.

- [ ] **Step 1: Delete the tracked backup**

Remove only `services/agent-gateway/src/context.rs.orig`; do not delete tests or historical design documents.

- [ ] **Step 2: Verify all retained test references are live**

Run: `rg -n 'test:realtime|test:mobile|smoke-test.py' apps/mobile/package.json deploy/agent-stack/README.md`

Expected: every retained test has a package script or deployment-document reference.

- [ ] **Step 3: Copy the focused smoke and cleanup changes to the remote checkout**

Run the remote standard-library contract suite and Python compilation from `/home/lake/workspace/ripple-live`.

- [ ] **Step 4: Run remote regression checks**

Run: `python3 deploy/agent-stack/smoke-test.py --responses-only`; then run the complete smoke only when `RIPPLE_SMOKE_ACCESS_TOKEN` is supplied in the remote shell environment.

Expected: responses-only PASS; full smoke validates authenticated voice/video behavior without emitting the token.
