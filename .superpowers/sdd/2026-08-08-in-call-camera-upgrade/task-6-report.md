# Task 6 report: safe in-call camera orchestration and 420 ms transition

## Outcome

Implemented the Android in-call voice/camera transition as one generation-owned transaction on the existing realtime session. The home camera entry now creates the normal audio session and opens the camera dynamically after media readiness; it does not create another WebSocket or conversation. Voice entry never requests camera access.

The App now exposes the strict `CameraPhase = 'off' | 'opening' | 'on' | 'closing' | 'error'`, a separately truthful preview flag, and frame-request state. The UI adds an accessible 44 px camera/retry control, limits flip to phase `on`, renders focus/“正在识别” only for an active frame request, and crossfades the warm-cobalt orb/camera over 420 ms (immediate under reduced motion).

## Behavioral TDD evidence

RED was recorded with `npm run test:media`: the new camera orchestration suite failed with `ERR_MODULE_NOT_FOUND` before production code existed.

GREEN covers the real asynchronous ordering and race behavior through injected media/session promises:

- first camera frame before `setMode('video')`, and phase `on` only after the matching acknowledgement;
- video failure/timeout attempts acknowledged audio correction before camera release;
- failed audio correction retains the camera, reports server mode `unknown`, and exposes close retry;
- close waits for audio acknowledgement, publishes the reverse visual state, waits 420 ms, then disables video;
- close failure retains the camera and retries safely;
- permission failure returns to the orb and exposes open retry;
- rapid double taps coalesce; invalidation makes late completions inert;
- camera interruption corrects audio best-effort without stopping microphone/playback.

Focused suite result: camera/media 34/34 passed (11 orchestration/lifecycle cases plus CameraController coverage).

## Ownership and failure rules

- A camera orchestrator instance belongs to one App session/media pair. Leave, session error/end, connection failure, logout, auth invalidation, component cleanup, and replacement invalidate its generation before resources are released.
- All App callbacks also require the existing live-call owner and exact session identity. Frame request callbacks cannot mutate a replacement call.
- A lost video acknowledgement may mean the server switched; therefore camera is disabled only after a corrective audio acknowledgement. If correction fails, the preview remains visible and the UI says the mode is unsynchronized instead of claiming audio/off.
- A close timeout leaves preview and retry available. A successful close switches the visual mode to audio before the 420 ms reverse transition, then releases video only.
- Camera track interruption invokes only mode correction. It does not stop LiveMedia audio, close RealtimeSession, or clear the conversation.
- Flip captures media/orchestrator/flip generations, commits facing state only for the latest phase-on result, and uses CameraController replacement without restarting audio.

## Files

- Created `apps/mobile/src/live/cameraOrchestration.ts`
- Created `apps/mobile/tests/camera-orchestration.test.ts`
- Modified `apps/mobile/src/App.tsx`
- Modified `apps/mobile/src/components/LiveCallScreen.tsx`
- Modified `apps/mobile/src/live/LiveCall.css`
- Modified `apps/mobile/src/media/LiveMedia.ts`
- Modified `apps/mobile/tests/live-media-lifecycle.test.ts`
- Modified `apps/mobile/tests/playback-telemetry.test.ts`
- Modified `apps/mobile/tests/mobile-package.test.mjs`
- Modified `apps/mobile/package.json`

`withVideo` was removed from App, LiveMedia, and media tests. `initialVideo` remains as the explicit legacy media API but is not used by App orchestration.

## Final verification

Fresh sequential verification completed with exit 0:

- `npm run test:live-media`: 34/34
- `npm run test:realtime`: 52/52
- `npm run test:mobile`: package/conversation/library suites passed
- `npm run test:media`: 11/11
- `npm run test:playback`: 5/5
- `npm run test:live-ui`: 23/23
- `npm run test:tool-results`: 44/44
- `npm run lint`: clean
- `npm run build`: TypeScript and Vite production build passed
- `git diff --check`: clean

Vite retains the existing warning that the main minified chunk exceeds 500 kB. No backend, Responses API, iOS, or generated native project files were changed.

## Independent-review fix round

Three Important findings were corrected test-first.

### Opening interruption serialization

An ended camera track could fire while `setMode('video')` was still pending. The first implementation invalidated the open owner but immediately called `setMode('audio')`; `RealtimeSession` correctly rejected that conflicting request as busy. `interrupt()` now captures and awaits the prior camera transaction (late video ACK, timeout, or error), then always issues the authoritative audio request if the same call generation still owns the correction.

Behavior tests cover late video ACK, video timeout, corrective-audio failure with unknown/retry state, successful audio retry without reopening camera, and call invalidation while the correction is queued. The microphone/playback/session are never stopped by camera interruption.

### Ready-gated camera control

`cameraControlReadyRef` is false for missing orchestration/media readiness and session states idle, connecting, preparing, ended, and error. The visible control is disabled with “准备中”, while the handler and flip handler independently guard the ref. It becomes true only after the owner session's `onReady` media start completes. The explicit home-video intent remains stored across startup, is consumed exactly once after readiness, and uses the same audio session.

All leave/error/logout/auth/session-replacement/unmount paths synchronously clear the ready ref and presentation state.

### Minimum frame-request visibility

Added `createMinimumVisibleSignal`, instantiated per owner session with injected timers. A true `onFrameRequestState` becomes visible immediately; the matching false schedules removal no earlier than 160 ms, so a capture queued within one task still crosses a paint window. A newer true cancels and extends the pending hide. Dispose cancels the timer and clears state; old session callbacks are inert through the existing owner check.

Only `onFrameRequestState` can assert the focus signal. Non-frame camera snapshots do not assert or prematurely clear it; lifecycle cleanup may synchronously clear it.

Review-fix focused GREEN:

- `npm run test:media`: 15/15
- `npm run test:live-ui`: 26/26, including 3 fake-timer visibility tests
- `npm run test:mobile`: all package/conversation/library checks
- `npm run build`: passed
- `npm run lint`: clean

The final fresh full sequence also passed: live-media 38/38, realtime 52/52, mobile package 18/18 plus conversation 6/6 and library 5/5, media 15/15, playback 5/5, live-ui 26/26, tool-results 44/44, lint, production build, and `git diff --check`.

## Re-review fix: late media-start activation

A further review identified that session identity alone was insufficient across the awaited `media.start()`: an ended/error session could still occupy `sessionRef` until its React effect ran, allowing a late successful media start to mark camera controls ready and consume the home-video intent.

Added a per-session `createCameraActivationGuard`. At call startup the entry intent is transferred into this guard and the global ref is cleared. `onReady` captures a token only in an allowed live state, awaits media startup, then requires all of the following before readiness or camera intent can commit:

- the same live-call owner and exact session;
- the same activation token in an allowed state;
- exact `mediaRef === media` ownership;
- exact `cameraOrchestratorRef === cameraOrchestrator` ownership.

Idle/connecting/preparing transitions during pending activation, ended/error, `onError`, connection failure, close/leave, logout, auth invalidation, replacement, and unmount invalidate the token and erase its per-session camera intent. A stale fulfillment therefore cannot enable the control or reacquire camera. A replacement session owns an independent guard.

Behavioral tests cover normal home-video one-time consumption, normal voice readiness without camera, pending start followed by ended, pending start followed by error, a pending disallowed-state transition, repeated commit rejection, and old/new session isolation.

Post-fix full verification passed: live-media 44/44, realtime 52/52, mobile package 18/18 plus conversation 6/6 and library 5/5, media 21/21, playback 5/5, live-ui 26/26, tool-results 44/44, lint, production build, and `git diff --check`.
