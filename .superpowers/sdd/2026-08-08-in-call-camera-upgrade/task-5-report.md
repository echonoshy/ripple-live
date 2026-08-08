# Task 5 Report: Dynamic camera lifecycle

## Result

- Local worktree: `/Users/lake/workspace/ripple-live/.worktrees/gpt-live-alignment`
- Base: `0e9a6141ae32e7e6fa9be435e14ee780454350bc`
- Android client media changes only; no App/UI, backend, generated Android, or iOS files changed.
- Added an independently injected `CameraController`, dynamic `LiveMedia.enableCamera()` / `disableCamera()`, and `cameraEnabled`.

## Implementation

- Camera acquisition occurs only after explicit `enable()`; `LiveMedia.start()` establishes playback, microphone capture, and its audio graph before the transitional `initialVideo` legacy entry can open the camera.
- Camera enable waits for an already playable frame or `loadeddata`, rejects on media error or a deterministic three-second timeout, and cleans listeners/timers on every success, error, timeout, or abort path.
- Operation generations and abortable first-frame waits make disable and overlapping enable/flip operations latest-wins. Late `getUserMedia` streams and stale first-frame completions are video-disposed and cannot reattach or re-enable the camera.
- Same-mode enable calls coalesce while pending and are idempotent after success. Requesting the currently active facing mode cancels an opposite pending flip and restores the current preview.
- A failed replacement restores the previous stream/transform and keeps its tracks alive. Successful replacement disposes only the previous video tracks.
- Disable is idempotent, clears `srcObject`, and stops only video tracks. It does not alter microphone capture, playback contexts, or audio tracks.
- Active camera `ended` / `mute` events disable the camera and invoke the optional `onCameraInterrupted` callback without ending audio, exposing rollback state for Task 6.
- `LiveMedia.stop()` and fatal start errors disable the camera. Frame capture now requires an actually enabled controller rather than the old fixed `withVideo` flag.
- The deprecated optional `withVideo` alias is retained only until Task 6 migrates the existing App constructor to `initialVideo`; the new dynamic API is already authoritative.

## TDD and race coverage

- RED was observed when `npm run test:live-media` failed because `CameraController.ts` did not exist.
- Added deterministic coverage for: no automatic camera request; exact environment constraints; first-frame gating; ready-state fast path; loadeddata/error/timeout/abort cleanup; pending acquisition and first-frame disable; same-mode coalescing; overlapping latest-wins; cancel-to-current behavior; failed-preview restoration; video-only disposal; track ended/mute interruption; and audio-first legacy startup.
- Self-review found and fixed two lifecycle defects before full verification: failure cleanup initially cleared `srcObject` before restoring the old preview, and a synchronous `loadeddata` from `play()` could leave a timeout installed.

## Verification

- `npm run test:live-media`: 26 passed after the independent-review follow-up.
- `npm run test:media`: 3 passed.
- `npm run test:mobile`: 17 package, 6 conversation-action, and 5 library tests passed.
- `npm run test:realtime`: 52 passed.
- `npm run test:playback`: 5 passed.
- `npm run test:live-ui`: 23 passed.
- `npm run test:tool-results`: 44 passed.
- `npm run lint`: passed.
- `npm run build`: passed; only the pre-existing Vite chunk-size advisory remains.
- `git diff --check`: passed.

## Independent-review follow-up

Two P1 lifecycle findings were reproduced with failing tests and fixed:

- A track `mute` event is no longer treated as an immediate terminal `ended` event. Mute now starts an injected, deterministic one-second grace timer; `unmute`, disable, a camera switch, an ended event, or listener replacement cancels it. Only a sustained mute interrupts once, and events from replaced tracks are inert.
- If the current track ends while a replacement is waiting for its first frame, one generation-safe interruption transaction now invalidates the operation, aborts the waiter, disposes every pending/current video stream, clears the preview, and reports once. Late first-frame resolution, rejection, or timeout resolves stale and cannot restore the ended stream or produce an unhandled rejection.

The focused suite now contains 26 passing tests, including transient/sustained mute, disable/switch during grace, old-track events, and all three late pending-frame outcomes.
