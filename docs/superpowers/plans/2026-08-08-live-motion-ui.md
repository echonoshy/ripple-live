# Ripple Live Realtime UI and Motion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current purple dashboard and bar visualizer with the approved four-tab, cold-blue, GPT Live-inspired Android experience and a real-event-driven C-tier soft-body motion system.

**Architecture:** Extract realtime presentation logic from `App.tsx` into pure motion state helpers and focused React components. Render one WebGL2 canvas for the soft-body core, feed it smoothed microphone and playback levels, and fall back to a static CSS core if graphics initialization fails. Keep the existing realtime transport and fixed start-time audio/video mode unchanged in this plan; in-call camera switching is added by the third plan.

**Tech Stack:** React 19, TypeScript 6, WebGL2, AudioWorklet, Tauri 2 Android, Node test runner, oxlint, Vite.

## Global Constraints

- Android APK is the only mobile implementation target.
- Do not modify, extend, or regenerate iOS files.
- Responses API remains the only allowed Agent API protocol.
- The camera never opens without an explicit user tap.
- Saving memory remains explicit; no automatic save prompts or automatic persistence.
- Use approximately 90ms press feedback, 280ms ordinary state transitions, 120–180ms interrupt release, 420ms camera transitions, and 1.8s caption hold.
- Target 60fps in high quality and degrade to 30fps after two seconds below 45fps or when reduced motion/power saving requires it.
- Do not add placeholder voice, intelligence-level, background-call, or continuous-video controls.
- Execute all three plans on the fixed branch `codex/gpt-live-alignment`.

## File Structure

- Create `apps/mobile/src/live/motion.ts`: visual-state mapping, level smoothing, quality policy, and timing constants.
- Create `apps/mobile/src/live/orbRenderer.ts`: the single-canvas WebGL2 renderer and CSS fallback contract.
- Create `apps/mobile/src/components/LiveOrb.tsx`: React lifecycle wrapper for `OrbRenderer`.
- Create `apps/mobile/src/components/LiveCaption.tsx`: one-line transient caption presentation.
- Create `apps/mobile/src/components/LiveCallScreen.tsx`: realtime call composition and controls.
- Create `apps/mobile/src/components/BottomNav.tsx`: four-tab navigation.
- Create `apps/mobile/src/components/ConversationHome.tsx`: minimal voice-first home.
- Create `apps/mobile/src/live/LiveCall.css`: call, orb, caption, video overlay, controls, and reduced-motion styles.
- Create `apps/mobile/src/components/AppNavigation.css`: home and bottom navigation styles.
- Create `apps/mobile/tests/live-motion.test.ts`: deterministic motion-policy tests.
- Modify `apps/mobile/public/playback-processor.js`: emit output RMS levels every 50ms.
- Modify `apps/mobile/src/media/LiveMedia.ts`: expose playback level and reset it on interruption/end.
- Modify `apps/mobile/src/App.tsx`: wire focused components and real input/output levels.
- Modify `apps/mobile/src/App.css`: remove superseded home/call rules and keep library/auth screens.
- Modify `apps/mobile/src/index.css`: apply cold-black global background and system typography.
- Modify `apps/mobile/tests/mobile-package.test.mjs`: replace obsolete purple/video-primary source assertions with new structural checks.
- Modify `apps/mobile/package.json`: add the focused live-motion test command.

---

### Task 1: Pure realtime presentation model

**Files:**
- Create: `apps/mobile/src/live/motion.ts`
- Create: `apps/mobile/tests/live-motion.test.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Consumes: `SessionState` from `src/realtime/RealtimeSession.ts`.
- Produces: `VisualState`, `MotionFrame`, `MOTION_TIMING`, `mapSessionState()`, `smoothLevel()`, and `nextQualityTier()`.

- [ ] **Step 1: Write the failing state and quality-policy tests**

```ts
import assert from 'node:assert/strict'
import test from 'node:test'
import {
  MOTION_TIMING,
  mapSessionState,
  nextQualityTier,
  smoothLevel,
} from '../src/live/motion.ts'

test('maps every transport state to one visual state', () => {
  assert.equal(mapSessionState('preparing'), 'connecting')
  assert.equal(mapSessionState('using_tool'), 'tool')
  assert.equal(mapSessionState('speaking'), 'speaking')
  assert.equal(mapSessionState('ended'), 'ended')
})

test('uses approved motion timing', () => {
  assert.deepEqual(MOTION_TIMING, {
    pressMs: 90,
    stateMs: 280,
    interruptMs: 160,
    cameraMs: 420,
    captionHoldMs: 1800,
  })
})

test('smooths level changes and clamps invalid input', () => {
  assert.equal(smoothLevel(0, 2, 0.5), 0.5)
  assert.equal(smoothLevel(1, -1, 0.5), 0.5)
})

test('degrades only after sustained slow frames and recovers with hysteresis', () => {
  assert.equal(nextQualityTier('high', 44, 2100, false), 'low')
  assert.equal(nextQualityTier('low', 58, 3000, false), 'low')
  assert.equal(nextQualityTier('low', 59, 6000, false), 'high')
  assert.equal(nextQualityTier('high', 60, 0, true), 'low')
})
```

- [ ] **Step 2: Add the test command and verify failure**

Add to `scripts`:

```json
"test:live-ui": "tsx --test tests/live-motion.test.ts"
```

Run: `cd apps/mobile && npm run test:live-ui`
Expected: FAIL because `src/live/motion.ts` does not exist.

- [ ] **Step 3: Implement the pure motion model**

```ts
import type { SessionState } from '../realtime/RealtimeSession'

export type VisualState =
  | 'idle' | 'connecting' | 'listening' | 'thinking'
  | 'tool' | 'speaking' | 'ended' | 'error'
export type QualityTier = 'high' | 'low'

export const MOTION_TIMING = {
  pressMs: 90,
  stateMs: 280,
  interruptMs: 160,
  cameraMs: 420,
  captionHoldMs: 1800,
} as const

const visualState: Record<SessionState, VisualState> = {
  idle: 'idle', connecting: 'connecting', preparing: 'connecting',
  listening: 'listening', thinking: 'thinking', using_tool: 'tool',
  speaking: 'speaking', ended: 'ended', error: 'error',
}

export const mapSessionState = (state: SessionState) => visualState[state]

export function smoothLevel(previous: number, input: number, alpha: number) {
  const target = Math.min(1, Math.max(0, Number.isFinite(input) ? input : 0))
  return previous + (target - previous) * Math.min(1, Math.max(0, alpha))
}

export function nextQualityTier(
  current: QualityTier,
  fps: number,
  stableForMs: number,
  forceLow: boolean,
): QualityTier {
  if (forceLow || (current === 'high' && fps < 45 && stableForMs >= 2000)) return 'low'
  if (current === 'low' && fps >= 58 && stableForMs >= 5000) return 'high'
  return current
}
```

- [ ] **Step 4: Run the focused tests**

Run: `cd apps/mobile && npm run test:live-ui`
Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/src/live/motion.ts apps/mobile/tests/live-motion.test.ts apps/mobile/package.json
git commit -m "feat(mobile): add realtime motion presentation model"
```

---

### Task 2: Playback-level telemetry for output-driven motion

**Files:**
- Modify: `apps/mobile/public/playback-processor.js`
- Modify: `apps/mobile/src/media/LiveMedia.ts`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: rendered PCM frames inside `StreamPlaybackProcessor.process()`.
- Produces: `LiveMediaOptions.onOutputLevel(level: number): void`; values are normalized to `0..1` and reset to `0` on clear/end.

- [ ] **Step 1: Add failing package assertions**

```js
const playbackSource = readFileSync(
  path.join(appRoot, 'public/playback-processor.js'), 'utf8',
)
assert.match(playbackSource, /type: 'audio-level'/)
assert.match(mediaSource, /onOutputLevel: \(level: number\) => void/)
assert.match(mediaSource, /this\.options\.onOutputLevel\(event\.data\.level\)/)
```

- [ ] **Step 2: Run the package test to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL on the missing `audio-level` event.

- [ ] **Step 3: Emit 20Hz RMS from the playback worklet**

Add `levelSquareSum`, `levelSampleCount`, and this block after samples are copied into `output`:

```js
for (const sample of output) this.levelSquareSum += sample * sample
this.levelSampleCount += output.length
if (this.levelSampleCount >= sampleRate / 20) {
  this.port.postMessage({
    type: 'audio-level',
    level: Math.min(1, Math.sqrt(this.levelSquareSum / this.levelSampleCount) * 6),
  })
  this.levelSquareSum = 0
  this.levelSampleCount = 0
}
```

Post `{ type: 'audio-level', level: 0 }` from the `clear` branch and when playback ends.

- [ ] **Step 4: Wire output levels through `LiveMedia`**

Extend the types and playback handler:

```ts
type LiveMediaOptions = {
  video: HTMLVideoElement
  canvas: HTMLCanvasElement
  withVideo: boolean
  facingMode: 'user' | 'environment'
  onPlaybackStarted: (bufferedMs: number) => void
  onPlaybackEnded: () => void
  onOutputLevel: (level: number) => void
}

type PlaybackStateMessage = {
  type: 'playback-started' | 'playback-ended' | 'playback-underrun' | 'audio-level'
  level?: number
  bufferedMs?: number
  count?: number
}

if (event.data.type === 'audio-level') {
  this.options.onOutputLevel(Math.min(1, Math.max(0, event.data.level ?? 0)))
  return
}
```

Call `onOutputLevel(0)` in `clearOutput()` and `stop()`.

Pass `onOutputLevel: setOutputLevel` from `App.tsx`; add `const [outputLevel, setOutputLevel] = useState(0)` now so this task compiles independently.

- [ ] **Step 5: Verify media tests and build**

Run: `cd apps/mobile && npm run test:mobile && npm run test:realtime && npm run build`
Expected: all commands PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/mobile/public/playback-processor.js apps/mobile/src/media/LiveMedia.ts apps/mobile/src/App.tsx apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): expose realtime playback levels"
```

---

### Task 3: Single-canvas soft-body renderer and fallback

**Files:**
- Create: `apps/mobile/src/live/orbRenderer.ts`
- Create: `apps/mobile/src/components/LiveOrb.tsx`
- Create: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `{ state, inputLevel, outputLevel, reducedMotion, qualityTier }`.
- Produces: `createOrbRenderer(canvas): OrbRenderer` with `update(frame)`, `resize()`, and `dispose()`.

- [ ] **Step 1: Add failing structural assertions**

```js
for (const file of ['live/orbRenderer.ts', 'components/LiveOrb.tsx', 'live/LiveCall.css']) {
  assert.equal(existsSync(path.join(appRoot, 'src', file)), true)
}
const orbSource = readFileSync(path.join(appRoot, 'src/components/LiveOrb.tsx'), 'utf8')
assert.equal((orbSource.match(/<canvas/g) ?? []).length, 1)
assert.doesNotMatch(orbSource, /lottie|video/i)
```

- [ ] **Step 2: Run package tests to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL because the renderer files are absent.

- [ ] **Step 3: Implement the renderer contract**

Use one WebGL2 context and one full-canvas fragment shader. Define the public API exactly as:

```ts
import type { QualityTier, VisualState } from './motion'

const STATE_INDEX: Record<VisualState, number> = {
  idle: 0, connecting: 1, listening: 2, thinking: 3,
  tool: 4, speaking: 5, ended: 6, error: 7,
}

const VERTEX_SHADER = `#version 300 es
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`

const FRAGMENT_SHADER = `#version 300 es
precision highp float;
uniform float uTime;
uniform float uInput;
uniform float uOutput;
uniform vec2 uResolution;
uniform int uState;
uniform int uQuality;
out vec4 outColor;
float ball(vec2 p, vec2 c, float r) {
  return r * r / max(dot(p - c, p - c), 0.002);
}
void main() {
  vec2 p = (gl_FragCoord.xy * 2.0 - uResolution) / uResolution.y;
  float energy = uState == 2 ? uInput : (uState == 5 ? uOutput : 0.16);
  float t = uTime * (0.42 + energy * 1.2);
  float field = ball(p, vec2(0.0), 0.48 + energy * 0.05);
  field += ball(p, vec2(cos(t), sin(t)) * 0.22, 0.25);
  field += ball(p, vec2(cos(t + 2.1), sin(t + 2.1)) * 0.20, 0.23);
  field += ball(p, vec2(cos(t + 4.2), sin(t + 4.2)) * 0.21, 0.24);
  if (uQuality == 1) {
    field += ball(p, vec2(cos(t * 0.7 + 1.0), sin(t * 0.7 + 1.0)) * 0.27, 0.15);
    field += ball(p, vec2(cos(t * 0.8 + 3.5), sin(t * 0.8 + 3.5)) * 0.26, 0.14);
  }
  float body = smoothstep(1.35, 1.65, field);
  float edge = smoothstep(1.05, 1.45, field) - body;
  float highlight = exp(-8.0 * dot(p - vec2(-0.18, 0.22), p - vec2(-0.18, 0.22)));
  vec3 deep = vec3(0.063, 0.231, 0.38);
  vec3 mid = vec3(0.298, 0.659, 0.886);
  vec3 ice = vec3(0.88, 0.97, 1.0);
  vec3 color = mix(deep, mid, clamp(field - 1.2, 0.0, 1.0));
  color = mix(color, ice, highlight * body * 0.85);
  outColor = vec4(color * body + mid * edge * 0.45, body + edge * 0.55);
}`

export type OrbFrame = {
  state: VisualState
  inputLevel: number
  outputLevel: number
  reducedMotion: boolean
  qualityTier: QualityTier
  nowMs: number
}

export type OrbRenderer = {
  update(frame: OrbFrame): void
  resize(width: number, height: number, pixelRatio: number): void
  dispose(): void
}

export function createOrbRenderer(canvas: HTMLCanvasElement): OrbRenderer {
  const gl = canvas.getContext('webgl2', { alpha: true, antialias: false })
  if (!gl) throw new Error('webgl2_unavailable')
  const compile = (kind: number, source: string) => {
    const shader = gl.createShader(kind)
    if (!shader) throw new Error('shader_create_failed')
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw new Error(gl.getShaderInfoLog(shader) ?? 'shader_compile_failed')
    }
    return shader
  }
  const program = gl.createProgram()
  if (!program) throw new Error('program_create_failed')
  gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX_SHADER))
  gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT_SHADER))
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) ?? 'program_link_failed')
  }
  gl.useProgram(program)
  const vao = gl.createVertexArray()
  gl.bindVertexArray(vao)
  const uniforms = {
    time: gl.getUniformLocation(program, 'uTime'),
    input: gl.getUniformLocation(program, 'uInput'),
    output: gl.getUniformLocation(program, 'uOutput'),
    state: gl.getUniformLocation(program, 'uState'),
    quality: gl.getUniformLocation(program, 'uQuality'),
    resolution: gl.getUniformLocation(program, 'uResolution'),
  }
  const update = (frame: OrbFrame) => {
    gl.uniform1f(uniforms.time, frame.nowMs / 1000)
    gl.uniform1f(uniforms.input, frame.inputLevel)
    gl.uniform1f(uniforms.output, frame.outputLevel)
    gl.uniform1i(uniforms.state, STATE_INDEX[frame.state])
    gl.uniform1i(uniforms.quality, frame.qualityTier === 'high' ? 1 : 0)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
  }
  const resize = (width: number, height: number, pixelRatio: number) => {
    canvas.width = Math.max(1, Math.round(width * pixelRatio))
    canvas.height = Math.max(1, Math.round(height * pixelRatio))
    gl.viewport(0, 0, canvas.width, canvas.height)
    gl.uniform2f(uniforms.resolution, canvas.width, canvas.height)
  }
  const dispose = () => {
    gl.deleteVertexArray(vao)
    gl.deleteProgram(program)
  }
  return { update, resize, dispose }
}
```

The shader must use signed-distance/metaball blending, cold-blue gradients, internal highlight, edge glow, and state-specific energy. Low quality uses fewer noise octaves and `min(devicePixelRatio, 1.25)`; high quality uses `min(devicePixelRatio, 2)`.

- [ ] **Step 4: Implement `LiveOrb` lifecycle and fallback**

```tsx
export type LiveOrbProps = {
  state: VisualState
  inputLevel: number
  outputLevel: number
}

export function LiveOrb(props: LiveOrbProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [fallback, setFallback] = useState(false)
  const latestProps = useRef({
    state: props.state,
    inputLevel: props.inputLevel,
    outputLevel: props.outputLevel,
    reducedMotion: false,
    qualityTier: 'high' as QualityTier,
  })
  latestProps.current.state = props.state
  latestProps.current.inputLevel = props.inputLevel
  latestProps.current.outputLevel = props.outputLevel
  useEffect(() => {
    if (!canvasRef.current) return
    let renderer: OrbRenderer
    try { renderer = createOrbRenderer(canvasRef.current) }
    catch { setFallback(true); return }
    const observer = new ResizeObserver(([entry]) => {
      const ratio = latestProps.current.qualityTier === 'high'
        ? Math.min(devicePixelRatio, 2)
        : Math.min(devicePixelRatio, 1.25)
      renderer.resize(entry.contentRect.width, entry.contentRect.height, ratio)
    })
    observer.observe(canvasRef.current)
    let frame = 0
    const draw = (nowMs: number) => {
      renderer.update({ ...latestProps.current, nowMs })
      frame = requestAnimationFrame(draw)
    }
    frame = requestAnimationFrame(draw)
    return () => { cancelAnimationFrame(frame); observer.disconnect(); renderer.dispose() }
  }, [])
  return fallback
    ? <div className={`live-orb-fallback is-${props.state}`} aria-hidden="true" />
    : <canvas ref={canvasRef} className="live-orb-canvas" aria-hidden="true" />
}
```

Keep `latestProps` in a ref updated on every render. Measure average frame time in rolling two-second windows and call `nextQualityTier`; recover only after five stable seconds. Set `forceLow` when reduced motion is enabled or, when `navigator.getBattery()` exists, battery is at or below 15% and not charging.

- [ ] **Step 5: Add fallback and reduced-motion styles**

Use `#020406` background, ice-white highlight, `#9edcff` body, `#4ca8e2` midtone, and `#103b61` shadow. Reduced motion disables displacement and retains only opacity/brightness changes.

Apply state transforms to the single canvas wrapper with the 280ms curve: connecting scale `0.88`, listening `1`, thinking `0.86`, tool `0.65` plus upward translation, speaking `1.04`, ended `0.75` plus opacity `0`, and error `0.9` with reduced saturation. The shader supplies continuous internal motion; CSS supplies only the semantic transition.

- [ ] **Step 6: Verify tests, lint, and build**

Run: `cd apps/mobile && npm run test:live-ui && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS; no new dependency is added.

- [ ] **Step 7: Commit**

```bash
git add apps/mobile/src/live apps/mobile/src/components/LiveOrb.tsx apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): add high fidelity live orb renderer"
```

---

### Task 4: Immersive call composition, transient captions, and natural interruption UI

**Files:**
- Create: `apps/mobile/src/components/LiveCaption.tsx`
- Create: `apps/mobile/src/components/LiveCallScreen.tsx`
- Modify: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: existing `SessionState`, `RealtimeMode`, `<video>`/capture `<canvas>` refs, real input/output levels, transcript text, artifacts, mute state, and callbacks.
- Produces: a presentation-only `LiveCallScreen`; session/media ownership remains in `App.tsx`.

- [ ] **Step 1: Add failing UI contract assertions**

```js
const callSource = readFileSync(path.join(appRoot, 'src/components/LiveCallScreen.tsx'), 'utf8')
assert.match(callSource, /<LiveOrb/)
assert.match(callSource, /<LiveCaption/)
assert.doesNotMatch(callSource, /HandPalm|打断回答/)
assert.match(callSource, /aria-label=\{muted \? '取消静音' : '静音'\}/)
assert.match(callSource, /aria-label="结束通话"/)
```

- [ ] **Step 2: Run package tests to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL because `LiveCallScreen.tsx` is absent.

- [ ] **Step 3: Implement transient caption selection**

```tsx
export function LiveCaption({ userText, assistantText, state }: LiveCaptionProps) {
  const text = state === 'speaking' ? assistantText : userText
  const [visible, setVisible] = useState(text)
  useEffect(() => {
    if (text) setVisible(text)
    if (!text) return
    const timer = window.setTimeout(() => setVisible(''), 1800)
    return () => window.clearTimeout(timer)
  }, [text])
  return <div className="live-caption" aria-live="polite">{visible}</div>
}
```

- [ ] **Step 4: Implement `LiveCallScreen` with exact prop boundary**

```ts
export type LiveCallScreenProps = {
  mode: RealtimeMode
  state: SessionState
  elapsed: number
  muted: boolean
  inputLevel: number
  outputLevel: number
  userText: string
  assistantText: string
  toolStatus: string
  errorMessage: string
  artifacts: ResponseArtifact[]
  server: string
  accessToken: string
  videoRef: RefObject<HTMLVideoElement | null>
  captureCanvasRef: RefObject<HTMLCanvasElement | null>
  onToggleMute(): void
  onFlipCamera(): Promise<void>
  onLeave(): Promise<void>
}
```

Audio mode centers `LiveOrb`; video mode uses a full-screen preview and never overlays the large orb. Replace the persistent transcript card with the state label and transient caption. Keep artifact images in a bottom sheet that does not stop playback.

- [ ] **Step 5: Wire input/output levels and natural interruption**

In `App.tsx`, store `inputLevel` and `outputLevel`, pass `onOutputLevel: setOutputLevel` to `LiveMedia`, and keep `speechStarted()` as the only normal interruption path. Delete the visible `forceListen()`/`HandPalm` control while retaining `forceListen()` internally for recovery and tests.

Replace the existing `visualizerRef.current?.style.setProperty(...)` input-level callback with `setInputLevel(level)`. Reset both levels to zero in `stopCall`, connection error handling, and the `ended` effect.

- [ ] **Step 6: Verify focused and regression tests**

Run: `cd apps/mobile && npm run test:realtime && npm run test:live-ui && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/mobile/src/components/LiveCaption.tsx apps/mobile/src/components/LiveCallScreen.tsx apps/mobile/src/live/LiveCall.css apps/mobile/src/App.tsx apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): adopt immersive realtime call UI"
```

---

### Task 5: Voice-first home and four-tab navigation

**Files:**
- Create: `apps/mobile/src/components/BottomNav.tsx`
- Create: `apps/mobile/src/components/ConversationHome.tsx`
- Create: `apps/mobile/src/components/AppNavigation.css`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/src/index.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `openCall('audio' | 'video')` and existing screen setters.
- Produces: `AppTab = 'chat' | 'memories' | 'todos' | 'profile'`; `BottomNav` is hidden only on auth and live-call screens.

- [ ] **Step 1: Replace obsolete home assertions with failing navigation assertions**

```js
const navSource = readFileSync(path.join(appRoot, 'src/components/BottomNav.tsx'), 'utf8')
for (const label of ['对话', '记忆', '待办', '我的']) assert.match(navSource, new RegExp(label))
assert.match(appSource, /<ConversationHome/)
assert.match(appSource, /<BottomNav/)
assert.doesNotMatch(appSource, /打开镜头，开始聊聊/)
assert.doesNotMatch(cssSource, /#9046ff|--ripple-violet|--voice-accent:\s*#b98aff/)
```

- [ ] **Step 2: Run package tests to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL on missing components and obsolete purple tokens.

- [ ] **Step 3: Implement minimal home**

`ConversationHome` renders one cold-blue core, “想聊点什么？”, a primary `开始说话` action, a lower-weight explicit camera action, and a compact history button. It does not render dashboard cards, statistics, recent conversations, or auto-save prompts.

- [ ] **Step 4: Implement bottom navigation**

```tsx
export type AppTab = 'chat' | 'memories' | 'todos' | 'profile'
export function BottomNav({ active, onSelect }: {
  active: AppTab
  onSelect(tab: AppTab): void
}) {
  const items: Array<{ tab: AppTab; label: string }> = [
    { tab: 'chat', label: '对话' },
    { tab: 'memories', label: '记忆' },
    { tab: 'todos', label: '待办' },
    { tab: 'profile', label: '我的' },
  ]
  return (
    <nav className="bottom-nav" aria-label="主导航">
      {items.map(({ tab, label }) => (
        <button
          key={tab}
          type="button"
          aria-current={active === tab ? 'page' : undefined}
          onClick={() => onSelect(tab)}
        >
          {label}
        </button>
      ))}
    </nav>
  )
}
```

Map tabs in `App.tsx`: chat → home, memories → memories, todos → todos, profile → settings. History and conversation detail retain the chat tab.

- [ ] **Step 5: Apply the cold-black theme and safe-area layout**

Set the global background to `#020406`; remove active purple tokens and call/home overrides; retain library contrast and red destructive actions. All touch targets remain at least 44×44 CSS pixels.

- [ ] **Step 6: Run the full mobile verification**

Run: `cd apps/mobile && npm run test:live-ui && npm run test:realtime && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/mobile/src/components/BottomNav.tsx apps/mobile/src/components/ConversationHome.tsx apps/mobile/src/components/AppNavigation.css apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/src/index.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): add voice first navigation and home"
```

---

### Task 6: Android motion QA and APK acceptance

**Files:**
- Modify only if QA finds a defect: files changed in Tasks 1–5.

**Interfaces:**
- Consumes: completed mobile UI branch.
- Produces: a verified Android APK with no iOS diff.

- [ ] **Step 1: Run all automated checks from a clean working tree**

```bash
cd apps/mobile
npm run test:live-ui
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
npm run android:build
```

Expected: all commands exit 0 and the APK is produced under `src-tauri/gen/android/app/build/outputs/apk/`.

- [ ] **Step 2: Verify the platform boundary**

Run: `git diff --name-only HEAD~5..HEAD -- apps/mobile/src-tauri/gen/apple apps/mobile/src-tauri/Info.ios.plist apps/mobile/src-tauri/tauri.ios.conf.json`
Expected: no output.

- [ ] **Step 3: Perform Android device interaction checks**

Verify: one-tap voice start; camera only after explicit tap; states map correctly; speaking uses output level; user speech clears audio and returns to listening within 180ms visually; captions fade after 1.8s; reduced-motion mode has no displacement; low quality holds 30fps; tool/image overlays do not stop audio; back gesture and safe areas work at 320px width and short screens.

- [ ] **Step 4: Commit QA-only corrections if any**

```bash
git add apps/mobile
git commit -m "fix(mobile): polish realtime motion QA"
```

Skip this commit if no files changed.
