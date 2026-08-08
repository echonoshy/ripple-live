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
