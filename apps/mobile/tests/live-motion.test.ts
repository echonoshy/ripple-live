import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import {
  captionTextForState,
  nextCaptionText,
  scheduleCaptionClear,
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

test('selects only the transcript for the active speaker state', () => {
  assert.equal(captionTextForState('speaking', '你好吗', '很好'), '很好')
  assert.equal(captionTextForState('listening', '你好吗', '旧回答'), '你好吗')
  assert.equal(captionTextForState('thinking', '', '旧回答'), '')
})

test('schedules caption clearing for 1800ms and cancels stale clears', () => {
  let callback: (() => void) | undefined
  let delay = 0
  let clearedTimer = 0
  const cancel = scheduleCaptionClear(
    () => { callback = undefined },
    {
      setTimeout(next, timeout) {
        callback = next
        delay = timeout
        return 7
      },
      clearTimeout(timer) {
        clearedTimer = timer
        callback = undefined
      },
    },
  )

  assert.equal(delay, 1800)
  assert.equal(typeof callback, 'function')
  cancel()
  assert.equal(clearedTimer, 7)
  assert.equal(callback, undefined)
})

test('clears unchanged text when the active caption source switches', () => {
  const previous = {
    source: 'user' as const,
    userText: '上一轮问题',
    assistantText: '上一轮回答',
  }

  assert.equal(nextCaptionText(previous, {
    source: 'assistant',
    userText: '上一轮问题',
    assistantText: '上一轮回答',
  }), '')
  assert.equal(nextCaptionText(previous, {
    source: 'assistant',
    userText: '上一轮问题',
    assistantText: '新的回答',
  }), '新的回答')
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
    captionHoldMs: 1800,
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

  assert.match(cssSource, /width:\s*188px/)
  assert.match(
    cssSource,
    /@keyframes live-orb-idle-breath\s*\{\s*from\s*\{[^}]*scale\(0\.965\)[^}]*\}\s*to\s*\{[^}]*scale\(1\.025\)/,
  )
  assert.match(cssSource, /animation:\s*live-orb-idle-breath 4\.2s/)
  assert.match(
    cssSource,
    /@keyframes live-orb-connecting\s*\{\s*from\s*\{[^}]*scale\(0\.92\)[^}]*\}\s*to\s*\{[^}]*scale\(0\.96\)/,
  )
  assert.match(
    cssSource,
    /@keyframes live-orb-thinking\s*\{\s*from\s*\{[^}]*scale\(0\.94\)[^}]*\}\s*to\s*\{[^}]*scale\(0\.98\)/,
  )
  assert.match(
    cssSource,
    /@keyframes live-orb-tool\s*\{\s*from\s*\{[^}]*translateY\(-32px\) scale\(0\.68\)[^}]*\}\s*to\s*\{[^}]*translateY\(-32px\) scale\(0\.72\)/,
  )
  assert.match(cssSource, /is-listening[^}]*scale\(var\(--live-input-scale\)\)/)
  assert.match(cssSource, /is-speaking[^}]*scale\(var\(--live-output-scale\)\)/)
  assert.match(
    cssSource,
    /@keyframes live-orb-interrupt-release\s*\{\s*0%\s*\{[^}]*scale\(1\.04\)[^}]*\}\s*55%\s*\{[^}]*scale\(0\.92\)[^}]*\}\s*100%\s*\{[^}]*scale\(1\)/,
  )
  assert.match(cssSource, /is-interruption-release[^}]*160ms/)
  assert.match(cssSource, /is-error[^}]*saturate\([^)]*\)[^}]*scale\(0\.92\)/)
  assert.match(callSource, /0\.98 \+ clampLevel\(inputLevel\) \* 0\.065/)
  assert.match(callSource, /0\.96 \+ clampLevel\(outputLevel\) \* 0\.115/)
})

test('uses the interruption release only for speaking-to-listening', () => {
  assert.equal(isInterruptionRelease('speaking', 'listening'), true)
  for (const previous of ['connecting', 'thinking', 'tool', 'listening'] as const) {
    assert.equal(isInterruptionRelease(previous, 'listening'), false)
  }
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
