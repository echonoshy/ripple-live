# Ripple Live In-Call Camera Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an Android user explicitly open or close the camera during an existing voice conversation, preserve the same conversation ID, and transition between the soft-body voice view and full-screen camera without claiming continuous video understanding.

**Architecture:** Introduce backward-compatible realtime protocol v5 mode-change events while continuing to accept v4 sessions. Deploy the compatible server first. Refactor `LiveMedia` so camera ownership is dynamic and independent from microphone/playback; the client opens the camera and waits for a playable first frame before requesting server video mode, and reverses the order safely when closing it.

**Tech Stack:** React 19, TypeScript 6, browser MediaDevices, Tauri 2 Android WebView, Rust 2024, Axum WebSocket, Node and Cargo tests.

## Global Constraints

- Android APK is the only mobile implementation target; do not modify or regenerate iOS.
- Responses API remains the only allowed Agent API protocol.
- Backend edits, tests, builds, and deployment run on `lake@140.143.229.103`.
- The server must accept both realtime protocol v4 and v5 during rollout; existing installed APKs must continue working.
- The camera starts only after an explicit user tap and successful permission grant.
- Opening or closing the camera does not create a new WebSocket or conversation ID.
- Video understanding remains server-requested JPEG frames for accepted turns; do not add continuous frame streaming.
- Show recognition/focus feedback only after `input.frame.requested`.
- Use the approved approximately 420ms visual transition after the first camera frame is ready.
- Permission denial, missing first frame, camera interruption, or mode-change failure returns to audio without ending the call.

## File Structure

- Modify remote `services/agent-gateway/src/protocol.rs`: `SessionMode` parsing and mode-change validation.
- Modify remote `services/agent-gateway/src/main.rs`: v4/v5 negotiation, `session.mode.set`, `session.mode.changed`, and pending-turn behavior.
- Modify `apps/mobile/src/realtime/protocol.ts`: protocol v5 mode-change event builder.
- Modify `apps/mobile/src/realtime/RealtimeSession.ts`: mode-change request/ack state and frame-request callback.
- Modify `apps/mobile/tests/realtime-session.test.ts`: v5 protocol, acknowledgement, stale mode event, and frame-request tests.
- Create `apps/mobile/src/media/CameraController.ts`: independently testable camera stream and first-frame lifecycle.
- Modify `apps/mobile/src/media/LiveMedia.ts`: dynamic `enableCamera()`, `disableCamera()`, `cameraEnabled`, and first-frame readiness.
- Create `apps/mobile/tests/live-media.test.ts`: mock MediaDevices camera lifecycle tests.
- Modify `apps/mobile/src/components/LiveCallScreen.tsx`: camera toggle, opening/closing visual states, and true frame-request focus state.
- Modify `apps/mobile/src/live/LiveCall.css`: 420ms orb/camera crossfade and failure rollback.
- Modify `apps/mobile/src/App.tsx`: orchestrate media-first open and server-first close.
- Modify `apps/mobile/tests/mobile-package.test.mjs`: no auto-camera and dynamic-camera assertions.

---

### Task 1: Backward-compatible server protocol contract

**Files:**
- Modify on remote host: `services/agent-gateway/src/protocol.rs`
- Modify on remote host: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Consumes: `session.start.protocol_version` equal to 4 or 5 and `session.mode.set` with `mode: "audio" | "video"` from v5 clients.
- Produces: `session.ready.protocol_version` echoing the negotiated version and `session.mode.changed` with the accepted mode.

- [ ] **Step 1: Confirm remote branch state**

Run:

```bash
ssh lake@140.143.229.103 'cd ~/workspace/ripple-live && git switch codex/gpt-live-alignment && git pull --ff-only && git status --short'
```

Expected: the remote checkout is clean on `codex/gpt-live-alignment`.

- [ ] **Step 2: Write failing protocol tests**

Replace the exact-v4 test with:

```rust
#[test]
fn realtime_protocol_accepts_v4_and_v5_during_rollout() {
    assert_eq!(validate_protocol_version(Some(4)), Ok(4));
    assert_eq!(validate_protocol_version(Some(5)), Ok(5));
    assert_eq!(validate_protocol_version(None), Err(()));
    assert_eq!(validate_protocol_version(Some(3)), Err(()));
    assert_eq!(validate_protocol_version(Some(6)), Err(()));
}

#[test]
fn session_mode_accepts_only_audio_and_video() {
    assert_eq!(SessionMode::parse(Some("audio")).unwrap(), SessionMode::Audio);
    assert_eq!(SessionMode::parse(Some("video")).unwrap(), SessionMode::Video);
    assert!(SessionMode::parse(Some("continuous_video")).is_err());
}
```

- [ ] **Step 3: Run focused tests to verify failure**

Run remotely:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml realtime_protocol_accepts_v4_and_v5_during_rollout
cargo test --manifest-path services/agent-gateway/Cargo.toml session_mode_accepts_only_audio_and_video
```

Expected: compile/test failure because negotiation and `SessionMode` are absent.

- [ ] **Step 4: Implement version and mode parsing**

In `protocol.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode { Audio, Video }

impl SessionMode {
    pub fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.unwrap_or("audio") {
            "audio" => Ok(Self::Audio),
            "video" => Ok(Self::Video),
            other => anyhow::bail!("不支持的会话模式: {other}"),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self { Self::Audio => "audio", Self::Video => "video" }
    }
}
```

In `main.rs`, set `REALTIME_PROTOCOL_MIN = 4`, `REALTIME_PROTOCOL_VERSION = 5`, and return the accepted version from `validate_protocol_version(version) -> Result<u32, ()>`. Replace the mutable mode `String` with `SessionMode`, update `queue_voice_transcript` and its callers to accept that enum, compare with `SessionMode::Video`, and serialize with `as_str()`.

- [ ] **Step 5: Echo negotiated version from `session.ready`**

Store `negotiated_protocol_version` per socket. v4 may initialize audio or video but sending `session.mode.set` returns a correlated `unsupported_protocol` error without closing the socket. v5 can change modes.

- [ ] **Step 6: Run server tests and commit**

```bash
cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml
cargo test --manifest-path services/agent-gateway/Cargo.toml
git add services/agent-gateway/src/protocol.rs services/agent-gateway/src/main.rs
git commit -m "feat(server): negotiate realtime protocol v5"
git push origin codex/gpt-live-alignment
```

Expected: all tests PASS and v4 remains accepted.

---

### Task 2: Server mode changes and pending video-turn safety

**Files:**
- Modify on remote host: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Consumes: v5 `session.mode.set`.
- Produces: `session.mode.changed`; subsequent accepted turns request a frame only in video mode.

- [ ] **Step 1: Write failing helper tests**

```rust
#[test]
fn switching_video_to_audio_releases_pending_turn_without_frames() {
    let pending = PendingTurn {
        response_id: "response-1".to_owned(),
        transcript: "继续回答".to_owned(),
    };
    let decision = plan_mode_change(SessionMode::Video, SessionMode::Audio, Some(&pending));
    assert_eq!(decision, ModeChangePlan::ReleasePendingAudioTurn);
}

#[test]
fn switching_audio_to_video_only_changes_future_turns() {
    assert_eq!(
        plan_mode_change(SessionMode::Audio, SessionMode::Video, None),
        ModeChangePlan::ChangeOnly,
    );
}
```

- [ ] **Step 2: Run focused tests to verify failure**

Run remotely:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml switching_video_to_audio_releases_pending_turn_without_frames
cargo test --manifest-path services/agent-gateway/Cargo.toml switching_audio_to_video_only_changes_future_turns
```

Expected: compile failure because `ModeChangePlan` does not exist.

- [ ] **Step 3: Implement the pure mode-change plan**

```rust
#[derive(Debug, PartialEq, Eq)]
enum ModeChangePlan { Unchanged, ChangeOnly, ReleasePendingAudioTurn }

fn plan_mode_change(
    current: SessionMode,
    requested: SessionMode,
    pending: Option<&PendingTurn>,
) -> ModeChangePlan {
    if current == requested { return ModeChangePlan::Unchanged; }
    if current == SessionMode::Video && requested == SessionMode::Audio && pending.is_some() {
        return ModeChangePlan::ReleasePendingAudioTurn;
    }
    ModeChangePlan::ChangeOnly
}
```

- [ ] **Step 4: Handle `session.mode.set` in the socket loop**

Validate v5, parse mode, and apply the plan. For `ReleasePendingAudioTurn`, drain frames, take `pending_turn`, and call existing `spawn_turn()` with an empty frame vector and the pending transcript. Then set `session_mode` and emit:

```rust
json!({
    "type": "session.mode.changed",
    "mode": session_mode.as_str()
})
```

Do not cancel an active response when mode changes. Clear stale stored frames whenever the accepted mode becomes audio.

- [ ] **Step 5: Add socket-level regression coverage**

Cover duplicate mode requests, invalid mode, v4 rejection without socket close, v5 audio→video acknowledgement, v5 video→audio release of a pending turn, and the rule that only later accepted video turns emit `input.frame.requested`.

- [ ] **Step 6: Run full server verification and commit**

```bash
cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml
cargo test --manifest-path services/agent-gateway/Cargo.toml
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
git add services/agent-gateway/src/main.rs
git commit -m "feat(server): switch camera mode inside realtime sessions"
git push origin codex/gpt-live-alignment
```

Expected: all commands PASS.

---

### Task 3: Deploy the compatible server before the v5 APK

**Files:**
- Remote deployment only.

**Interfaces:**
- Consumes: server commits from Tasks 1–2.
- Produces: production server that accepts existing v4 APKs and the upcoming v5 APK.

- [ ] **Step 1: Build and restart on the remote host**

```bash
ssh lake@140.143.229.103 '
  cd ~/workspace/ripple-live &&
  git pull --ff-only &&
  cargo build --release --manifest-path services/agent-gateway/Cargo.toml &&
  ./deploy/agent-stack/stop.sh &&
  ./deploy/agent-stack/start.sh &&
  ./deploy/agent-stack/status.sh
'
```

Expected: Gateway is active and readiness is healthy.

- [ ] **Step 2: Run production smoke tests**

Run remotely: `uv run --with httpx deploy/agent-stack/smoke-test.py`
Expected: all existing v4 realtime and Responses API smoke checks PASS.

- [ ] **Step 3: Manually verify a currently installed v4 APK**

Start one voice turn and one video turn. Expected: both connect and complete normally before any v5 client is shipped.

---

### Task 4: Client protocol v5 mode-change state machine

**Files:**
- Modify: `apps/mobile/src/realtime/protocol.ts`
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Modify: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- Consumes: `setMode(mode: RealtimeMode)` calls and `session.mode.changed` events.
- Produces: protocol v5 `createModeSet(mode)`, serialized mode changes, and `onModeChanged(mode)`.

- [ ] **Step 1: Sync the compatible server commits to the local branch**

Run: `git fetch origin && git switch codex/gpt-live-alignment && git merge --ff-only origin/codex/gpt-live-alignment`
Expected: local branch contains the deployed v4/v5 server implementation and is clean.

- [ ] **Step 2: Write failing protocol and session tests**

```ts
test('protocol v5 creates a mode-set event', () => {
  assert.equal(REALTIME_PROTOCOL_VERSION, 5)
  assert.deepEqual(createModeSet('video'), { type: 'session.mode.set', mode: 'video' })
})

test('setMode resolves only after matching acknowledgement', async () => {
  const { session, receive, sent } = readySessionHarness()
  const changed = session.setMode('video')
  assert.deepEqual(sent.at(-1), { type: 'session.mode.set', mode: 'video' })
  receive({ type: 'session.mode.changed', mode: 'video' })
  await changed
})

test('a stale mode acknowledgement does not resolve the request', async () => {
  const { session, receive } = readySessionHarness()
  const changed = session.setMode('video')
  receive({ type: 'session.mode.changed', mode: 'audio' })
  const outcome = await Promise.race([changed.then(() => 'resolved'), new Promise((r) => setTimeout(() => r('waiting'), 10))])
  assert.equal(outcome, 'waiting')
  receive({ type: 'session.mode.changed', mode: 'video' })
  await changed
})
```

- [ ] **Step 3: Run realtime tests to verify failure**

Run: `cd apps/mobile && npm run test:realtime`
Expected: FAIL because protocol v5 and `setMode()` are absent.

- [ ] **Step 4: Implement protocol and acknowledgement handling**

```ts
export const REALTIME_PROTOCOL_VERSION = 5
export const createModeSet = (mode: RealtimeMode) => ({
  type: 'session.mode.set' as const,
  mode,
})
```

`RealtimeSession.setMode(mode)` allows one pending request, sends with high priority, and rejects after 5 seconds or on close/error. `session.mode.changed` resolves only the matching request and invokes `onModeChanged(mode)`.

- [ ] **Step 5: Add frame-request visibility callback**

Add `onFrameRequestState(active: boolean)`; set true immediately before capture, and false after `createRequestedFrameEvents()` has been queued or capture throws. This callback drives the temporary focus frame and “正在识别” status.

- [ ] **Step 6: Run realtime tests and build**

Run: `cd apps/mobile && npm run test:realtime && npm run build`
Expected: all tests and build PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/mobile/src/realtime/protocol.ts apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/tests/realtime-session.test.ts
git commit -m "feat(mobile): negotiate in-call camera mode"
```

---

### Task 5: Dynamic camera lifecycle in `LiveMedia`

**Files:**
- Create: `apps/mobile/src/media/CameraController.ts`
- Modify: `apps/mobile/src/media/LiveMedia.ts`
- Create: `apps/mobile/tests/live-media.test.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Consumes: explicit `enableCamera(facingMode)` and `disableCamera()`.
- Produces: `CameraController.enabled`, first-frame resolution, dynamic camera switching, and unchanged audio playback/capture through `LiveMedia`.

- [ ] **Step 1: Write failing lifecycle tests with mocked MediaDevices**

```ts
import assert from 'node:assert/strict'
import test from 'node:test'
import { CameraController } from '../src/media/CameraController.ts'

function createCameraHarness() {
  const videoTrack = { stopCalls: 0, stop() { this.stopCalls += 1 } }
  const stream = { getTracks: () => [videoTrack] } as unknown as MediaStream
  const video = { srcObject: null } as unknown as HTMLVideoElement
  const constraints: MediaStreamConstraints[] = []
  const controller = new CameraController(video, {
    getUserMedia: async (request) => { constraints.push(request); return stream },
    waitForFirstFrame: async () => {},
  })
  return { constraints, controller, video, videoTrack }
}

test('enableCamera requests environment video and waits for first frame', async () => {
  const { constraints, controller } = createCameraHarness()
  await controller.enable('environment')
  assert.deepEqual(constraints[0], {
    audio: false,
    video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
  })
  assert.equal(controller.enabled, true)
})

test('disableCamera stops only video tracks', async () => {
  const { controller, video, videoTrack } = createCameraHarness()
  await controller.enable('environment')
  controller.disable()
  assert.equal(videoTrack.stopCalls, 1)
  assert.equal(video.srcObject, null)
})
```

- [ ] **Step 2: Add the test command and verify failure**

Add:

```json
"test:live-media": "tsx --test tests/live-media.test.ts"
```

Run: `cd apps/mobile && npm run test:live-media`
Expected: FAIL because the dynamic methods are absent.

- [ ] **Step 3: Implement `CameraController` with injected dependencies**

```ts
export type CameraDependencies = {
  getUserMedia(constraints: MediaStreamConstraints): Promise<MediaStream>
  waitForFirstFrame(video: HTMLVideoElement, timeoutMs: number): Promise<void>
}

export class CameraController {
  private stream: MediaStream | null = null
  constructor(private video: HTMLVideoElement, private deps: CameraDependencies) {}
  get enabled() { return this.stream !== null }
  async enable(facingMode: 'user' | 'environment') {
    if (this.stream) return
    const stream = await this.deps.getUserMedia({
      audio: false,
      video: { facingMode: { ideal: facingMode }, width: { ideal: 1280 }, height: { ideal: 720 } },
    })
    try {
      this.video.srcObject = stream
      await this.deps.waitForFirstFrame(this.video, 3000)
      this.stream = stream
    } catch (error) {
      stream.getTracks().forEach((track) => track.stop())
      this.video.srcObject = null
      throw error
    }
  }
  disable() {
    this.stream?.getTracks().forEach((track) => track.stop())
    this.stream = null
    this.video.srcObject = null
  }
}
```

- [ ] **Step 4: Remove fixed `withVideo` ownership from `LiveMedia`**

Keep `initialVideo: boolean` only for the legacy video entry during migration. `start()` initializes microphone/playback first, then calls `enableCamera()` only when `initialVideo` is true and the user reached that entry by tapping the camera action.

- [ ] **Step 5: Compose the controller in `LiveMedia`**

```ts
get cameraEnabled() { return this.camera.enabled }

async enableCamera(facingMode = this.facingMode) {
  this.facingMode = facingMode
  await this.camera.enable(facingMode)
}

disableCamera() {
  this.camera.disable()
}
```

Production dependencies call `navigator.mediaDevices.getUserMedia` and resolve `waitForFirstFrame` on `loadeddata` or reject after 3000ms. `setFacingMode()` disables and re-enables only when `cameraEnabled` is true.

- [ ] **Step 6: Run tests and commit**

Run: `cd apps/mobile && npm run test:live-media && npm run test:mobile && npm run build`
Expected: all commands PASS.

```bash
git add apps/mobile/src/media/CameraController.ts apps/mobile/src/media/LiveMedia.ts apps/mobile/tests/live-media.test.ts apps/mobile/package.json
git commit -m "feat(mobile): open camera inside an active call"
```

---

### Task 6: Orchestrate safe open/close and the 420ms transition

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/components/LiveCallScreen.tsx`
- Modify: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `LiveMedia.enableCamera/disableCamera`, `RealtimeSession.setMode`, and `onFrameRequestState`.
- Produces: `cameraPhase = 'off' | 'opening' | 'on' | 'closing' | 'error'` and explicit camera controls.

- [ ] **Step 1: Add failing App contract assertions**

```js
assert.match(appSource, /type CameraPhase = 'off' \| 'opening' \| 'on' \| 'closing' \| 'error'/)
assert.match(appSource, /await media\.enableCamera/)
assert.match(appSource, /await session\.setMode\('video'\)/)
assert.match(appSource, /await session\.setMode\('audio'\)/)
assert.doesNotMatch(mediaSource, /if \(this\.options\.withVideo\) await this\.openCamera\(\)/)
```

- [ ] **Step 2: Run package tests to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL on missing camera phase and orchestration.

- [ ] **Step 3: Implement open ordering**

On explicit camera tap: set `opening`; call `media.enableCamera(cameraFacing)`; after first frame, call `session.setMode('video')`; after acknowledgement, set `mode` to video and `cameraPhase` to on. If either call fails, call `media.disableCamera()`, keep server/audio mode, set a recoverable error message, and return to off after the message is shown.

- [ ] **Step 4: Implement close ordering**

Set `closing`; await `session.setMode('audio')`; set visual mode to audio; start the 420ms reverse transition; then call `media.disableCamera()` and set off. If server mode change fails, leave the camera on and show retry instead of creating split client/server state.

- [ ] **Step 5: Render truthful visual states**

The focus frame and “正在识别” appear only while `frameRequestActive` is true. “镜头已开启” appears only in phase on. Opening shows “正在开启镜头”; errors return to the orb. The orb and camera use opacity/scale transitions totaling 420ms; `prefers-reduced-motion` reduces this to an immediate crossfade.

- [ ] **Step 6: Verify all mobile checks and commit**

Run: `cd apps/mobile && npm run test:live-media && npm run test:realtime && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/components/LiveCallScreen.tsx apps/mobile/src/live/LiveCall.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): transition between voice and camera live modes"
```

---

### Task 7: Android APK and production acceptance

**Files:**
- Modify only if QA reveals a defect: files from Tasks 4–6.

**Interfaces:**
- Consumes: production-compatible v5 server and completed Android client.
- Produces: verified APK and an end-to-end in-call camera flow.

- [ ] **Step 1: Run clean automated verification**

```bash
cd apps/mobile
npm run test:live-ui
npm run test:tool-results
npm run test:live-media
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
npm run android:build
```

Expected: all commands exit 0.

- [ ] **Step 2: Confirm no iOS changes**

Run: `git diff --name-only master...HEAD -- apps/mobile/src-tauri/gen/apple apps/mobile/src-tauri/Info.ios.plist apps/mobile/src-tauri/tauri.ios.conf.json`
Expected: no output.

- [ ] **Step 3: Run Android device scenarios**

Verify: voice starts without camera permission; explicit camera tap prompts once and keeps the same conversation; permission denial stays in audio; first frame transitions in 420ms; later accepted video turns show the focus frame only during the server request; closing camera preserves active audio and conversation ID; switching while a frame is pending still produces one response; rotating front/back does not restart audio; network/mode failure leaves client and server on the last acknowledged mode.

- [ ] **Step 4: Re-run remote smoke after APK verification**

Run remotely: `uv run --with httpx deploy/agent-stack/smoke-test.py`
Expected: all production smoke checks PASS and v4 compatibility remains intact.
