# Warm GPT Live-aligned Android UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Ripple Live's cold, container-heavy Android UI with the approved compact warm-cobalt visual system, a stable fluid orb, and a low-frequency single-ring Ripple response across all nine main screens.

**Architecture:** Keep `RealtimeSession`, Responses API transport, conversation ownership, tool results, and existing data mutations unchanged. Add one pure Ripple scheduler between real UI events and the existing single WebGL2 canvas, replace the metaball shader with a stable circular fluid field, and restyle focused React presentation components plus the existing screen markup in `App.tsx`. Finish the already-negotiated realtime v5 camera mode on the mobile client so the compact camera control is truthful and never opens without a tap.

**Tech Stack:** React 19, TypeScript 6, WebGL2/GLSL ES 3.0, Phosphor Icons 2, Tauri 2 Android, Node test runner, oxlint, Vite 8.

## Global Constraints

- Android APK is the only implementation target; do not modify, extend, regenerate, build, or format iOS files.
- Responses API remains the only allowed Agent API protocol.
- Backend changes are outside this plan; the existing realtime protocol v5 mode negotiation is consumed as-is.
- The camera starts only after an explicit user tap and successful permission grant.
- Do not add non-functional voice, intelligence-level, background-call, continuous-video, attachment, or text-input controls.
- Use `--live-bg: #07080C`, `--app-bg: #09090B`, `--surface: #101014`, `--surface-raised: #151821`, `--text-primary: #F5F4F0`, `--danger: #ED687A`, and `--success: #69D49D`.
- The orb palette is `#0A2E75`, `#2F77E6`, `#9BC3FF`, and `#FFF6E9`; `#FFE5DC` and `#C0C9FF` together remain below 8% of a frame.
- The orb silhouette stays circular; do not render concentric outlines, a fixed inner circle, loading rings, audio bars, or large jelly deformation.
- Use a 4%–6% near halo and at most one Ripple ring from `1.03×` to `1.28×` radius over 700ms, with at least 1200ms between triggers.
- Target 60fps in high quality, 30fps in low quality, `devicePixelRatio <= 2`, downgrade after two seconds below 45fps, and recover after five seconds at or above 58fps.
- Every visible icon uses the Phosphor regular line style, visual size 18/20/22px, and a 44–50px tap target.
- Preserve real memory, todo, result, history, rename, delete, notification, and camera behaviors; do not fabricate success states.

## File Structure

- Create `apps/mobile/src/live/ripple.ts`: pure event scheduling, cooldown, single-ring concurrency, assistant emphasis gating, and reduced-motion pulse state.
- Create `apps/mobile/tests/ripple-motion.test.ts`: deterministic Ripple scheduler tests.
- Modify `apps/mobile/src/live/orbRenderer.ts`: stable circular B1/B2 fluid shader, near halo, single event ring, and renderer uniforms.
- Modify `apps/mobile/src/live/orbLifecycle.ts`: own the Ripple scheduler and pass one computed frame to the renderer.
- Modify `apps/mobile/src/components/LiveOrb.tsx`: accept real event signals and expose the same renderer on home and call screens.
- Create `apps/mobile/src/media/CameraController.ts`: explicit camera stream and first-frame lifecycle for the compact camera control.
- Create `apps/mobile/tests/live-media.test.ts`: dynamic camera lifecycle coverage.
- Modify `apps/mobile/src/media/LiveMedia.ts`: compose dynamic camera ownership without restarting microphone or playback.
- Modify `apps/mobile/src/components/ConversationHome.tsx`: real idle orb, compact copy, icon-only history and camera actions.
- Modify `apps/mobile/src/components/BottomNav.tsx`: consistent regular-line icons and a restrained active indicator.
- Modify `apps/mobile/src/components/LiveCallScreen.tsx`: compact top identity, unboxed captions, camera/mic/end controls, results and video states.
- Modify `apps/mobile/src/components/LiveResultSheet.tsx`: warm-neutral result hierarchy with existing controlled result types.
- Modify `apps/mobile/src/components/LibraryToolbar.tsx`: compact search/filter/selection presentation without changing mutations.
- Modify `apps/mobile/src/components/AppNavigation.css`: home and bottom-nav layout.
- Modify `apps/mobile/src/live/LiveCall.css`: orb, Ripple, live call, video, result, and reduced-motion styles.
- Modify `apps/mobile/src/App.css`: design tokens and history/detail/memory/todo/profile styles.
- Modify `apps/mobile/src/index.css`: neutral root background and typography.
- Modify `apps/mobile/src/App.tsx`: emit real Ripple signals, orchestrate camera transitions, shorten copy, and adjust supporting-screen markup.
- Modify `apps/mobile/tests/live-motion.test.ts`, `apps/mobile/tests/orb-renderer.test.ts`, `apps/mobile/tests/orb-lifecycle.test.ts`, and `apps/mobile/tests/mobile-package.test.mjs`: behavior and structural regressions.
- Modify `apps/mobile/package.json`: include the new focused tests in `test:live-ui` and `test:live-media`.

---

### Task 1: Pure low-frequency Ripple scheduler

**Files:**
- Create: `apps/mobile/src/live/ripple.ts`
- Create: `apps/mobile/tests/ripple-motion.test.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Consumes: `RippleSignal`, `VisualState`, output RMS, current time, and reduced-motion preference.
- Produces: `RIPPLE_MOTION`, `createRippleState()`, and `advanceRipple(state, input, nowMs) -> { state, frame }` where `frame` contains `progress`, `alpha`, and `haloPulse`.

- [ ] **Step 1: Write failing scheduler tests**

```ts
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  RIPPLE_MOTION,
  advanceRipple,
  createRippleState,
  type RippleSignal,
} from '../src/live/ripple.ts'

const signal = (id: number, kind: RippleSignal['kind']): RippleSignal => ({ id, kind })

test('uses the approved B/R2 dimensions and timing', () => {
  assert.deepEqual(RIPPLE_MOTION, {
    durationMs: 700,
    cooldownMs: 1200,
    startRadius: 1.03,
    endRadius: 1.28,
    maximumAlpha: 0.14,
  })
})

test('consumes dense events but starts only one ring inside the cooldown', () => {
  let state = createRippleState()
  let next = advanceRipple(state, { signal: signal(1, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false }, 1000)
  state = next.state
  assert.equal(next.frame.progress, 0)
  next = advanceRipple(state, { signal: signal(2, 'tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 1300)
  assert.equal(next.frame.kind, 'speech')
  assert.ok(next.frame.progress > 0)
  next = advanceRipple(next.state, { signal: signal(3, 'tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 2300)
  assert.equal(next.frame.kind, 'tool')
})

test('assistant output crosses the emphasis threshold once per speaking phrase', () => {
  let state = createRippleState()
  let next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.31, reducedMotion: false }, 2000)
  state = next.state
  assert.equal(next.frame.kind, 'assistant')
  next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.82, reducedMotion: false }, 3300)
  assert.equal(next.frame.kind, null)
  state = advanceRipple(next.state, { signal: null, visualState: 'listening', outputLevel: 0, reducedMotion: false }, 3400).state
  next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.31, reducedMotion: false }, 4600)
  assert.equal(next.frame.kind, 'assistant')
})

test('reduced motion suppresses propagation and keeps a short halo pulse', () => {
  const next = advanceRipple(createRippleState(), { signal: signal(1, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: true }, 1000)
  assert.equal(next.frame.progress, null)
  assert.ok(next.frame.haloPulse > 0)
})
```

- [ ] **Step 2: Add test commands and verify failure**

Set scripts to:

```json
"test:live-ui": "tsx --test tests/live-motion.test.ts tests/ripple-motion.test.ts tests/orb-lifecycle.test.ts tests/orb-renderer.test.ts",
"test:live-media": "tsx --test tests/live-media.test.ts tests/live-media-lifecycle.test.ts"
```

Run: `cd apps/mobile && npm run test:live-ui`

Expected: FAIL because `src/live/ripple.ts` does not exist.

- [ ] **Step 3: Implement the scheduler**

Use these public types and thresholds:

```ts
import type { VisualState } from './motion'

export type RippleKind = 'speech' | 'assistant' | 'tool' | 'interrupt'
export type RippleSignal = { id: number; kind: RippleKind }
export type RippleInput = {
  signal: RippleSignal | null
  visualState: VisualState
  outputLevel: number
  reducedMotion: boolean
}
export type RippleFrame = {
  kind: RippleKind | null
  progress: number | null
  alpha: number
  haloPulse: number
}
export const RIPPLE_MOTION = {
  durationMs: 700,
  cooldownMs: 1200,
  startRadius: 1.03,
  endRadius: 1.28,
  maximumAlpha: 0.14,
} as const
```

`advanceRipple()` must consume each signal ID once, reject starts before `cooldownUntilMs`, keep the active kind until 700ms completes, use `1 - progress` squared for alpha decay, arm assistant emphasis only after leaving `speaking`, and use `outputLevel >= 0.28` as the single emphasis onset. A suppressed signal may increase `haloPulse` but may not replace or stack the active ring.

- [ ] **Step 4: Run focused tests**

Run: `cd apps/mobile && npm run test:live-ui`

Expected: all live UI tests PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/src/live/ripple.ts apps/mobile/tests/ripple-motion.test.ts apps/mobile/package.json
git commit -m "feat(mobile): add low-frequency ripple scheduler"
```

---

### Task 2: Stable warm fluid orb and one-canvas Ripple rendering

**Files:**
- Modify: `apps/mobile/src/live/orbRenderer.ts`
- Modify: `apps/mobile/src/live/orbLifecycle.ts`
- Modify: `apps/mobile/src/components/LiveOrb.tsx`
- Modify: `apps/mobile/tests/orb-renderer.test.ts`
- Modify: `apps/mobile/tests/orb-lifecycle.test.ts`

**Interfaces:**
- Consumes: `RippleSignal | null` from React and `advanceRipple()` from Task 1.
- Produces: `OrbFrame.rippleProgress`, `OrbFrame.rippleAlpha`, `OrbFrame.haloPulse`, and a single shader pass containing core, halo, and ring.

- [ ] **Step 1: Add failing renderer and lifecycle tests**

Extend `OrbFrame` fixtures with `rippleProgress: null`, `rippleAlpha: 0`, and `haloPulse: 0`. Assert that the fragment shader declares `uRippleProgress`, `uRippleAlpha`, and `uHaloPulse`; uses `#0A2E75`, `#2F77E6`, `#9BC3FF`, and `#FFF6E9` converted to normalized RGB; does not contain the old `ball(` metaball helper; and keeps `uRippleProgress` at `-1` under reduced motion. In the lifecycle harness, send speech signals with IDs 1 and 2 at 1000ms and 1300ms and assert only the first produces a non-negative Ripple progress.

- [ ] **Step 2: Run tests to verify failure**

Run: `cd apps/mobile && npm run test:live-ui`

Expected: FAIL on missing Ripple uniforms and the old metaball shader.

- [ ] **Step 3: Replace the fragment field with a stable circular material**

Use a fixed radial signed-distance mask and keep motion inside it:

```glsl
vec2 uv = (gl_FragCoord.xy * 2.0 - uResolution) / uResolution.y;
float radius = 0.52;
float distanceToCore = length(uv);
float coreMask = 1.0 - smoothstep(radius - 0.012, radius + 0.008, distanceToCore);
float slowTime = uReducedMotion == 1 ? 0.0 : uTime;
float cloud = fbm(uv * 2.7 + vec2(slowTime * 0.07, -slowTime * 0.05));
float ribbon = fbm(uv * 4.1 + vec2(-slowTime * 0.11, slowTime * 0.08));
vec3 deep = vec3(0.039, 0.180, 0.459);
vec3 cobalt = vec3(0.184, 0.467, 0.902);
vec3 softBlue = vec3(0.608, 0.765, 1.0);
vec3 cream = vec3(1.0, 0.965, 0.914);
vec3 dawn = mix(vec3(1.0, 0.898, 0.863), vec3(0.753, 0.788, 1.0), 0.45);
vec3 color = mix(deep, cobalt, smoothstep(0.22, 0.82, cloud));
color = mix(color, softBlue, smoothstep(0.58, 0.92, ribbon) * 0.42);
color = mix(color, cream, highlight * 0.58);
color = mix(color, dawn, dawnReflection * 0.08);
```

High quality evaluates five octaves; low quality evaluates three. State and smoothed energy alter cloud speed, brightness, and CSS scale, never the circular SDF radius by more than the approved scale ranges.

- [ ] **Step 4: Render near halo and the single outward ring in the same pass**

Use eased progress and a widening band:

```glsl
float halo = exp(-52.0 * pow(max(distanceToCore - radius, 0.0), 2.0));
float p = clamp(uRippleProgress, 0.0, 1.0);
float eased = 1.0 - pow(1.0 - p, 3.0);
float ringRadius = radius * mix(1.03, 1.28, eased);
float ringWidth = mix(0.010, 0.038, p);
float ring = exp(-pow((distanceToCore - ringRadius) / ringWidth, 2.0));
float haloAlpha = mix(0.04, 0.06, clamp(uEnergy + uHaloPulse, 0.0, 1.0));
float ringAlpha = uRippleProgress < 0.0 ? 0.0 : ring * uRippleAlpha;
```

Composite premultiplied color and alpha once. Low quality uses the same radial ring without angular noise; reduced motion passes `-1` and only raises `uHaloPulse`.

- [ ] **Step 5: Integrate scheduling into the lifecycle**

Add optional `rippleSignal?: RippleSignal | null` to `LiveOrbProps` and required `rippleSignal: RippleSignal | null` to `OrbLifecycleState.current`; the wrapper stores `props.rippleSignal ?? null`. `startOrbLifecycle()` owns one `RippleState`, calls `advanceRipple()` before `renderer.update()`, and passes the computed values. Preserve existing quality cadence, observer cleanup, fallback, and context-loss behavior.

- [ ] **Step 6: Update the CSS fallback**

Use the same stable B1 palette and a pseudo-element near halo. The fallback may briefly brighten for a signal but must never draw repeating rings. Preserve all existing state classes and reduced-motion behavior.

- [ ] **Step 7: Verify and commit**

Run: `cd apps/mobile && npm run test:live-ui && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/live/orbRenderer.ts apps/mobile/src/live/orbLifecycle.ts apps/mobile/src/components/LiveOrb.tsx apps/mobile/tests/orb-renderer.test.ts apps/mobile/tests/orb-lifecycle.test.ts
git commit -m "feat(mobile): render warm fluid orb and event ripple"
```

---

### Task 3: Compact home, typography, icons, and bottom navigation

**Files:**
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/src/index.css`
- Modify: `apps/mobile/src/components/ConversationHome.tsx`
- Modify: `apps/mobile/src/components/BottomNav.tsx`
- Modify: `apps/mobile/src/components/AppNavigation.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `LiveOrb` from Task 2 and the existing home callbacks.
- Produces: the approved A2.2 home screen and the shared B1/B2 design tokens used by later tasks.

- [ ] **Step 1: Replace obsolete home assertions with failing approved-copy and structure assertions**

Assert that home contains `有什么想聊的？`, `可以直接说`, `开始对话`, one `<LiveOrb state="idle"`, an icon-only history button whose visible `历史` span is absent, and an icon-only camera button with `aria-label="打开镜头"`. Assert every bottom-navigation icon uses `weight="regular"`, and the old filled active weight and `.conversation-core` markup are absent.

- [ ] **Step 2: Run the package test to verify failure**

Run: `cd apps/mobile && npm run test:mobile`

Expected: FAIL on the old home copy and CSS core.

- [ ] **Step 3: Install exact shared tokens and typography**

Replace `:root` aliases with the values in Global Constraints plus:

```css
--line: rgb(255 255 255 / 8%);
--text-secondary: rgb(238 237 232 / 58%);
--text-tertiary: rgb(238 237 232 / 36%);
--orb-deep: #0a2e75;
--orb-cobalt: #2f77e6;
--orb-soft-blue: #9bc3ff;
--orb-cream: #fff6e9;
--focus-ring: rgb(155 195 255 / 58%);
```

Use `Inter, "SF Pro Display", "PingFang SC", "Noto Sans SC", system-ui, sans-serif`, `letter-spacing: -0.04em` only for headings, and no negative tracking on body copy.

- [ ] **Step 4: Rebuild the home composition**

Place the 156dp `LiveOrb` in the upper half, heading 38dp below it, `可以直接说` as secondary copy, a 183×47dp cream `开始对话` button, and a separate 47dp camera action. Keep the history action at the top right with an 18px icon inside a 44px target.

- [ ] **Step 5: Normalize bottom navigation**

Render all four Phosphor icons with `weight="regular"`, 20px visual size, and 44px targets. Active state uses `--orb-soft-blue`, slightly brighter label text, and one 3px dot; remove filled icon variants and selected background blocks.

- [ ] **Step 6: Verify and commit**

Run: `cd apps/mobile && npm run test:mobile && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/App.css apps/mobile/src/index.css apps/mobile/src/components/ConversationHome.tsx apps/mobile/src/components/BottomNav.tsx apps/mobile/src/components/AppNavigation.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): refine home and navigation hierarchy"
```

---

### Task 4: Truthful dynamic camera lifecycle

**Files:**
- Create: `apps/mobile/src/media/CameraController.ts`
- Create: `apps/mobile/tests/live-media.test.ts`
- Modify: `apps/mobile/src/media/LiveMedia.ts`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: the existing `RealtimeSession.setMode('audio' | 'video')` acknowledgement API.
- Produces: `LiveMedia.enableCamera()`, `disableCamera()`, `cameraEnabled`, and App-level `cameraPhase: 'off' | 'opening' | 'on' | 'closing'`.

- [ ] **Step 1: Write failing camera lifecycle tests**

Test that `enable('environment')` requests video-only media, waits for `loadeddata` before resolving, and exposes `enabled = true`; `disable()` stops video tracks and clears `video.srcObject`; a first-frame timeout stops the pending stream; and calling `disable()` never stops the separate audio stream owned by `LiveMedia`.

- [ ] **Step 2: Run focused tests to verify failure**

Run: `cd apps/mobile && npm run test:live-media`

Expected: FAIL because `CameraController.ts` does not exist.

- [ ] **Step 3: Implement independent camera ownership**

Create `CameraController(video, dependencies)` with injected `getUserMedia` and `waitForFirstFrame`. `enable()` uses `{ audio: false, video: { facingMode: { ideal }, width: { ideal: 1280 }, height: { ideal: 720 } } }`, rejects after 3000ms without a playable frame, and disposes stale generations. `disable()` is idempotent. `setFacingMode()` replaces video only when enabled.

- [ ] **Step 4: Compose the controller in `LiveMedia`**

Replace fixed camera ownership with:

```ts
get cameraEnabled() { return this.camera.enabled }
async enableCamera(facingMode = this.facingMode) {
  this.facingMode = facingMode
  await this.camera.enable(facingMode)
}
disableCamera() { this.camera.disable() }
```

Keep `initialVideo` solely for the explicit home camera entry. Microphone, VAD, capture worklet, and playback remain active when the camera toggles.

- [ ] **Step 5: Orchestrate acknowledged open and close in App**

Opening order is camera first-frame -> `session.setMode('video')` -> set `mode` and phase `on`. On failure, disable camera, remain in audio, and show a recoverable camera error. Closing order is `session.setMode('audio')` -> set visual mode audio -> disable camera -> phase `off`; if acknowledgement fails, leave the camera on and show retry. Never create a new socket or conversation ID.

- [ ] **Step 6: Verify and commit**

Run: `cd apps/mobile && npm run test:live-media && npm run test:realtime && npm run test:mobile && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/media/CameraController.ts apps/mobile/tests/live-media.test.ts apps/mobile/src/media/LiveMedia.ts apps/mobile/src/App.tsx apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): add explicit in-call camera lifecycle"
```

---

### Task 5: Compact live call layout and real Ripple event wiring

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/components/LiveCallScreen.tsx`
- Modify: `apps/mobile/src/components/LiveCaption.tsx`
- Modify: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/tests/live-motion.test.ts`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: camera lifecycle from Task 4 and `RippleSignal` from Task 1.
- Produces: compact audio/video call UI and exactly one signal for speech start, tool result, and completed interruption.

- [ ] **Step 1: Write failing call structure and label tests**

Assert that the old `.call-status` container, `.call-mode` line, visible control labels, `PhoneDisconnect`, and large `.call-controls` container styling are absent. Assert the header contains a collapse icon, `Ripple`, elapsed time, and one contextual action; controls contain camera, microphone, and an `X` end button; state copy maps `listening` to `我在听`, `thinking` to `想一想`, errors to `连接断开`, and speaking has no persistent `正在回答` label.

- [ ] **Step 2: Run tests to verify failure**

Run: `cd apps/mobile && npm run test:live-ui && npm run test:mobile`

Expected: FAIL on the old status pill and labels.

- [ ] **Step 3: Emit Ripple signals only from real events**

Add App state `const [rippleSignal, setRippleSignal] = useState<RippleSignal | null>(null)` and an incrementing ref. Emit `speech` inside the existing VAD `onSpeechStart`, `tool` inside `onToolResult`, and `interrupt` inside `onInterrupted` after `media.clearOutput()`. Do not emit from `onLevel`, transcript deltas, or every session-state update. Pass the signal through `LiveCallScreen` to `LiveOrb`; assistant emphasis remains scheduler-driven from output RMS.

- [ ] **Step 4: Recompose the header, stage, captions, and controls**

Use a three-column header: collapse button; centered `Ripple` plus tabular elapsed time; flip-camera or more action. Center the 188dp orb slightly above vertical center. Place the 10–12sp state text and 15–17sp latest caption directly on the background. Render three separate 50px buttons at the bottom with no enclosing card: camera, microphone, and a danger `X`. Keep all tap targets at least 44px and preserve accessible labels.

- [ ] **Step 5: Apply exact state scale ranges**

Idle breath uses `0.965 -> 1.025` over 4.2s; connecting stays `0.92–0.96`; listening `0.98–1.045`; thinking `0.94–0.98`; tool translates up 32dp and scales `0.68–0.72`; speaking `0.96–1.075`; interrupt releases `1.04 -> 0.92 -> 1` over 160ms; error holds `0.92` with reduced saturation. Input drives listening and output RMS drives speaking.

- [ ] **Step 6: Polish video truth states**

Camera preview fills the screen. Display `正在开启镜头` only during `opening`, `镜头已开启` only in phase `on`, and recognition focus only during the existing server frame-request callback. Use a 420ms crossfade after the first frame; reduced motion uses an immediate opacity crossfade.

- [ ] **Step 7: Verify and commit**

Run: `cd apps/mobile && npm run test:live-ui && npm run test:live-media && npm run test:realtime && npm run test:mobile && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/components/LiveCallScreen.tsx apps/mobile/src/components/LiveCaption.tsx apps/mobile/src/live/LiveCall.css apps/mobile/tests/live-motion.test.ts apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): tighten live call and wire ripple events"
```

---

### Task 6: Warm result cards and live-stage transitions

**Files:**
- Modify: `apps/mobile/src/components/LiveResultSheet.tsx`
- Modify: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/tests/tool-results.test.ts`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: existing controlled `LiveResult` variants and artifact data.
- Produces: compact bottom result sheets without changing result parsing or success/error truth.

- [ ] **Step 1: Add failing structural regression tests**

Assert result cards still support weather, search sources, memory receipts, todo receipts, image artifacts, and errors. Assert they use 18–22px regular icons, keep dismiss buttons at 44px, and do not introduce generic JSON rendering or success copy for error results.

- [ ] **Step 2: Run tests to verify the old visual contract fails**

Run: `cd apps/mobile && npm run test:tool-results && npm run test:mobile`

Expected: structural result tests PASS and new approved class/token assertions FAIL.

- [ ] **Step 3: Restyle the result system**

Enter the tray from the bottom over 280ms. Use `--surface-raised`, an 8% white line, 18–20px radii, 14px content padding, primary 14px copy, 10–12px metadata, B1 blue icons, `--success` only on confirmed receipts, and `--danger` only on failures. Remove cyan card borders and blue-tinted card backgrounds.

- [ ] **Step 4: Keep the call stable while results appear**

Move the orb stage up approximately 32dp and scale it to `0.70` for tool/result mode without moving the bottom controls. Limit the tray to 42dvh, preserve scrolling and source-button accessibility, and keep video results above the control safe area.

- [ ] **Step 5: Verify and commit**

Run: `cd apps/mobile && npm run test:tool-results && npm run test:mobile && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/components/LiveResultSheet.tsx apps/mobile/src/live/LiveCall.css apps/mobile/tests/tool-results.test.ts apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): refine realtime result presentation"
```

---

### Task 7: History and conversation detail hierarchy

**Files:**
- Modify: `apps/mobile/src/App.tsx:1397-1590,1848-1925`
- Modify: `apps/mobile/src/components/LibraryToolbar.tsx`
- Modify: `apps/mobile/src/components/LibrarySection.tsx`
- Modify: `apps/mobile/src/components/LibraryActions.tsx`
- Modify: `apps/mobile/src/components/AppNavigation.css`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/library.test.mjs`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: current history queries, selection, swipe, rename/delete, actions, and voice continuation callbacks.
- Produces: compact history and conversation-detail screens with unchanged behavior.

- [ ] **Step 1: Add failing page hierarchy assertions**

Assert history has a 22–24sp title, icon search affordance, compact query field, title-first rows with muted preview/time, and a 48dp voice floating action. Assert assistant messages are unboxed, user messages use one low-contrast rounded surface, tool actions remain compact, and the bottom continuation action starts audio for the same conversation ID.

- [ ] **Step 2: Run library and package tests to verify failure**

Run: `cd apps/mobile && npm run test:library && npm run test:mobile`

Expected: existing data tests PASS and new hierarchy assertions FAIL.

- [ ] **Step 3: Tighten history markup and CSS**

Keep the existing loading, error, empty, selection, pin/archive, swipe, rename, and delete flows. Reduce row height and padding, remove nested card borders, give title the only strong weight, place time at row end in tertiary ink, clamp preview to one line, and add a functional voice FAB that calls `openCall('audio')`.

- [ ] **Step 4: Tighten conversation detail**

Make Ripple messages borderless on the page background. Keep user messages on `--surface` with an 18px radius. Reduce metadata contrast, retain Markdown and actual memory/todo actions, and replace the large top continuation button with a compact bottom continuation bar whose only interactive action is the existing voice continuation. Do not render unsupported attachment or free-form text controls.

- [ ] **Step 5: Verify and commit**

Run: `cd apps/mobile && npm run test:library && npm run test:mobile && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/components/LibraryToolbar.tsx apps/mobile/src/components/LibrarySection.tsx apps/mobile/src/components/LibraryActions.tsx apps/mobile/src/components/AppNavigation.css apps/mobile/src/App.css apps/mobile/tests/library.test.mjs apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): refine conversation library screens"
```

---

### Task 8: Memories, todos, profile, and dialogs

**Files:**
- Modify: `apps/mobile/src/App.tsx:1592-1847,1927-1953,1984-2043`
- Modify: `apps/mobile/src/components/LibraryToolbar.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/library.test.mjs`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: existing memory/todo/account state and mutations.
- Produces: the remaining approved main screens and consistent sheets/dialogs without new fake settings.

- [ ] **Step 1: Add failing structural assertions**

Assert memories use a tight two-column grid with image, title, note/time hierarchy; todos expose `进行中 / 已完成`, put due time at row end, and use strike-through plus reduced opacity for completed items; profile groups the real account, service connection, notification state, voice/caption behavior, memory entry, data-use explanation, and logout action; dialogs use the shared warm-neutral surface and danger token.

- [ ] **Step 2: Run tests to verify failure**

Run: `cd apps/mobile && npm run test:library && npm run test:mobile`

Expected: existing behavior tests PASS and new visual structure assertions FAIL.

- [ ] **Step 3: Rework memories without changing persistence**

Keep all/pinned/archived data operations and selection behavior. Present all and pinned as primary compact filters and archive inside the overflow action; render cover-first two-column cards with 8px gaps, 14–16px titles, 10–12px time, restrained pin markers, and a bottom detail sheet using the same surface/radius system.

- [ ] **Step 4: Rework todos without changing completion semantics**

Rename the active tab label to `进行中`; remove the long intro paragraph; keep search, add, edit, swipe-delete, due/overdue, complete/restore, and cover behavior. Use thin row separators instead of heavy cards, a 44px circular completion target, one-line title, secondary summary, and tertiary due time. Completed rows use 52% opacity and line-through title text.

- [ ] **Step 5: Build a truthful profile overview**

Show the real user email, fixed connected service endpoint, `Notification.permission` state, captions described as automatic during live calls, a navigation row to existing memories, a concise microphone/camera data-use explanation, and the existing logout action. Render informational rows as text, not toggles. Do not add unavailable voice selection, intelligence level, background call, or continuous video settings.

- [ ] **Step 6: Unify sheets and destructive dialogs**

Use `--surface`, `--surface-raised`, 8% lines, 20–24px radii, 44px buttons, and `--danger`. Preserve `role`, labels, focus visibility, backdrop dismissal rules, save/rename disabled states, and deletion wording.

- [ ] **Step 7: Verify and commit**

Run: `cd apps/mobile && npm run test:library && npm run test:mobile && npm run lint && npm run build`

Expected: all commands PASS.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/components/LibraryToolbar.tsx apps/mobile/src/App.css apps/mobile/tests/library.test.mjs apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): unify supporting screens and dialogs"
```

---

### Task 9: Android performance, visual, and regression acceptance

**Files:**
- Verify: files changed in Tasks 1–8.
- Produce locally: Android APK and nine baseline screenshots; do not commit generated build output.

**Interfaces:**
- Consumes: completed UI, renderer, camera, and existing realtime behaviors.
- Produces: a verified Android artifact with recorded visual evidence.

- [ ] **Step 1: Run the full clean mobile suite**

```bash
cd apps/mobile
npm run test:live-ui
npm run test:tool-results
npm run test:live-media
npm run test:realtime
npm run test:media
npm run test:playback
npm run test:mobile
npm run lint
npm run build
npm run android:build
```

Expected: every command exits 0; Vite may report the existing non-fatal chunk-size warning.

- [ ] **Step 2: Verify platform and protocol boundaries**

Run:

```bash
git diff --name-only 84ee831...HEAD -- apps/mobile/src-tauri/gen/apple apps/mobile/src-tauri/Info.ios.plist apps/mobile/src-tauri/tauri.ios.conf.json
rg -n "chat/completions|assistants|threads/runs" apps/mobile/src services/agent-gateway/src
```

Expected: both commands produce no matches/output relevant to a violation.

- [ ] **Step 3: Install on the connected Android device**

Run `adb devices` and confirm device `80e0a09e`, then install the debug or signed APK produced by `android:build`. Launch from a clean app task while preserving account data unless an authentication test explicitly requires logout.

- [ ] **Step 4: Record the eight orb states**

Capture idle, connecting, listening, thinking, speaking, tool/result, interruption, and error. Confirm a stable circle, B1 cobalt material, cream highlight, less than 8% dawn reflection, real energy response, approved scale ranges, no concentric rings, and no fixed strong animation.

- [ ] **Step 5: Record Ripple frequency and performance**

Speak continuously for at least ten seconds and play one multi-sentence answer. Confirm one speech-onset ring, at most one assistant ring per speaking phrase, no stacked rings, no ring during the 1200ms cooldown, 700ms disappearance, 4%–6% near halo, and no per-syllable triggering. Measure high quality near 60fps; force sustained slow frames to verify 30fps fallback and recovery hysteresis.

- [ ] **Step 6: Verify camera and result truth**

Start audio without a camera prompt; tap camera to open it; deny permission once and confirm audio continues; allow permission and confirm the 420ms transition; flip camera; close camera without changing conversation ID; verify focus feedback only on a server frame request; trigger a real result, memory receipt, todo receipt, and error result without false success styling.

- [ ] **Step 7: Capture all nine main screens**

Capture home, audio call, realtime result, video call, history, conversation detail, memories, todos, and profile at the device's native viewport. Confirm 44px tap targets, safe-area spacing, compact copy, regular-line icons, no oversized pills, and consistent warm-neutral backgrounds.

- [ ] **Step 8: Verify reduced motion and narrow/short layouts**

Enable Android reduced motion, repeat speech and result events, and confirm only halo brightness changes with no outward propagation. Check widths around 320dp and heights below 700dp: no clipped controls, overlapped captions, hidden result dismiss buttons, or bottom-nav collisions.

- [ ] **Step 9: Commit any final acceptance-only test corrections**

If a deterministic test needed correction to match verified behavior, stage only that test and its owning implementation file, rerun the full command block from Step 1, and commit:

```bash
git commit -m "fix(mobile): resolve warm live UI acceptance regressions"
```

If no correction was needed, do not create an empty commit.
