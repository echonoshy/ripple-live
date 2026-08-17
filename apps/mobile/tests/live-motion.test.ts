import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import * as motionModule from '../src/live/motion.ts'
import {
  emptyUserCaption,
  nextUserCaption,
} from '../src/live/caption.ts'
import {
  cameraErrorAfterSwitch,
  visibleCallError,
} from '../src/live/callErrors.ts'
import {
  MOTION_TIMING,
  isInterruptionRelease,
  mapSessionState,
  nextQualityTier,
  smoothLevel,
} from '../src/live/motion.ts'

test('shows only the user question and hides all assistant speech text', () => {
  const question = '你再给我讲一个长一点的笑话。'
  const listening = nextUserCaption(emptyUserCaption, 'listening', question)
  assert.deepEqual(listening, { text: question, responseActive: false })

  const thinking = nextUserCaption(listening, 'thinking', question)
  assert.deepEqual(thinking, { text: question, responseActive: true })

  const speaking = nextUserCaption(thinking, 'speaking', question)
  assert.deepEqual(speaking, { text: '', responseActive: true })

  const resumed = nextUserCaption(speaking, 'listening', question)
  assert.deepEqual(resumed, emptyUserCaption)
})

test('camera outcomes cannot clear or mask a session error that arrives mid-switch', () => {
  let sessionError = ''
  let cameraError = '上次摄像头切换失败'

  cameraError = ''
  sessionError = '实时连接已断开'
  cameraError = cameraErrorAfterSwitch(cameraError, 'switched')
  assert.equal(sessionError, '实时连接已断开')
  assert.equal(cameraError, '')
  assert.equal(visibleCallError(sessionError, cameraError), '实时连接已断开')

  cameraError = cameraErrorAfterSwitch(cameraError, 'failed')
  assert.equal(sessionError, '实时连接已断开')
  assert.equal(cameraError, '无法切换摄像头，请重试')
  assert.equal(visibleCallError(sessionError, cameraError), '实时连接已断开')
})

test('camera success clears only prior camera-owned feedback', () => {
  assert.equal(cameraErrorAfterSwitch('上次摄像头切换失败', 'switched'), '')
  assert.equal(
    cameraErrorAfterSwitch('上次摄像头切换失败', 'stale'),
    '上次摄像头切换失败',
  )
  assert.equal(visibleCallError('', '无法切换摄像头，请重试'), '无法切换摄像头，请重试')
})

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
  })
})

test('live orb uses the compact state scale ranges and RMS inputs', () => {
  const cssSource = readFileSync(
    new URL('../src/live/LiveCall.css', import.meta.url),
    'utf8',
  )
  const callSource = readFileSync(
    new URL('../src/components/LiveCallScreen.tsx', import.meta.url),
    'utf8',
  )

  assert.match(cssSource, /width:\s*232px/)
  assert.match(
    cssSource,
    /@keyframes live-orb-idle-breath\s*\{\s*from\s*\{[^}]*scale\(0\.992\)[^}]*\}\s*to\s*\{[^}]*scale\(1\.008\)/,
  )
  assert.match(cssSource, /animation:\s*live-orb-idle-breath 7\.2s/)
  assert.match(cssSource, /is-listening[^}]*scale\(1\)/)
  assert.match(cssSource, /is-idle,[\s\S]*is-speaking[\s\S]*animation:\s*live-orb-idle-breath 7\.2s/)
  assert.doesNotMatch(callSource, /--live-(?:input|output)-scale/)
  assert.match(
    cssSource,
    /@keyframes live-orb-interrupt-release\s*\{\s*0%\s*\{[^}]*scale\(1\.012\)[^}]*\}\s*55%\s*\{[^}]*scale\(0\.985\)[^}]*\}\s*100%\s*\{[^}]*scale\(1\)/,
  )
  assert.match(cssSource, /is-interruption-release[^}]*160ms/)
  assert.match(cssSource, /is-error[^}]*saturate\([^)]*\)[^}]*scale\(0\.92\)/)
  assert.doesNotMatch(callSource, /clampLevel|--live-(?:input|output)-scale/)
})

test('uses the interruption release only for speaking-to-listening', () => {
  assert.equal(isInterruptionRelease('speaking', 'listening'), true)
  for (const previous of ['connecting', 'thinking', 'tool', 'listening'] as const) {
    assert.equal(isInterruptionRelease(previous, 'listening'), false)
  }
})

test('holds interruption release through RMS renders for the full 160ms', () => {
  const createLatch = Reflect.get(
    motionModule,
    'createInterruptionReleaseLatch',
  ) as undefined | ((
    onActive: (active: boolean) => void,
    timers: {
      setTimeout(callback: () => void, delayMs: number): number
      clearTimeout(handle: number): void
    },
  ) => {
    update(previous: 'speaking' | 'listening', next: 'speaking' | 'listening'): void
    current(): boolean
    dispose(): void
  })
  assert.equal(typeof createLatch, 'function')
  if (!createLatch) return

  let callback: (() => void) | null = null
  let delayMs = 0
  const cleared: number[] = []
  const changes: boolean[] = []
  const latch = createLatch((active) => changes.push(active), {
    setTimeout(next, delay) {
      callback = next
      delayMs = delay
      return 17
    },
    clearTimeout(handle) {
      cleared.push(handle)
      callback = null
    },
  })

  latch.update('speaking', 'listening')
  assert.equal(latch.current(), true)
  assert.equal(delayMs, 160)
  assert.deepEqual(changes, [true])

  latch.update('listening', 'listening')
  assert.equal(latch.current(), true, 'an RMS-only render must not release the latch')
  assert.ok(callback)
  callback()
  assert.equal(latch.current(), false)
  assert.deepEqual(changes, [true, false])

  latch.dispose()
  assert.deepEqual(cleared, [])
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

test('keeps video camera truth secondary to required session copy', async () => {
  let presentationModule: Record<string, unknown> = {}
  try {
    presentationModule = await import('../src/live/callPresentation.ts')
  } catch {
    // The RED state has no presentation policy module yet.
  }
  const labelsFor = Reflect.get(presentationModule, 'liveCallLabels') as
    | undefined
    | ((state: string, cameraPhase: string, toolStatus: string) => {
        primary: string
        camera: string
      })
  assert.equal(typeof labelsFor, 'function')
  if (!labelsFor) return

  assert.deepEqual(labelsFor('listening', 'on', ''), {
    primary: '',
    camera: '镜头已开启',
  })
  assert.deepEqual(labelsFor('thinking', 'on', ''), {
    primary: '想一想',
    camera: '镜头已开启',
  })
  assert.deepEqual(labelsFor('error', 'on', ''), {
    primary: '连接断开',
    camera: '镜头已开启',
  })
  assert.deepEqual(labelsFor('listening', 'opening', ''), {
    primary: '',
    camera: '正在开启镜头',
  })
  assert.deepEqual(labelsFor('speaking', 'on', ''), {
    primary: '',
    camera: '镜头已开启',
  })
})

test('provides a truthful camera header action for every phase', async () => {
  let presentationModule: Record<string, unknown> = {}
  try {
    presentationModule = await import('../src/live/callPresentation.ts')
  } catch {
    // The RED state has no presentation policy module yet.
  }
  const actionFor = Reflect.get(presentationModule, 'cameraHeaderAction') as
    | undefined
    | ((phase: string, previewVisible: boolean, ready: boolean) => {
        kind: 'toggle' | 'flip'
        label: string
        disabled: boolean
      })
  assert.equal(typeof actionFor, 'function')
  if (!actionFor) return

  assert.deepEqual(actionFor('off', false, true), {
    kind: 'toggle', label: '开启镜头', disabled: false,
  })
  assert.deepEqual(actionFor('opening', false, true), {
    kind: 'toggle', label: '正在开启镜头', disabled: true,
  })
  assert.deepEqual(actionFor('on', true, true), {
    kind: 'flip', label: '切换摄像头', disabled: false,
  })
  assert.deepEqual(actionFor('closing', true, true), {
    kind: 'toggle', label: '正在关闭镜头', disabled: true,
  })
  assert.deepEqual(actionFor('error', false, true), {
    kind: 'toggle', label: '重试镜头', disabled: false,
  })
  assert.deepEqual(actionFor('error', true, true), {
    kind: 'toggle', label: '重试关闭镜头', disabled: false,
  })
  assert.deepEqual(actionFor('off', false, false), {
    kind: 'toggle', label: '镜头尚未就绪', disabled: true,
  })
})
