import type { SessionState } from '../realtime/RealtimeSession'

export type VisualState =
  | 'idle'
  | 'connecting'
  | 'listening'
  | 'thinking'
  | 'tool'
  | 'speaking'
  | 'ended'
  | 'error'

export type QualityTier = 'high' | 'low'

export type MotionFrame = {
  state: VisualState
  level: number
  quality: QualityTier
}

export const MOTION_TIMING = {
  pressMs: 90,
  stateMs: 280,
  interruptMs: 160,
  cameraMs: 420,
} as const

const visualState: Record<SessionState, VisualState> = {
  idle: 'idle',
  connecting: 'connecting',
  preparing: 'connecting',
  listening: 'listening',
  thinking: 'thinking',
  using_tool: 'tool',
  speaking: 'speaking',
  ended: 'ended',
  error: 'error',
}

export const mapSessionState = (state: SessionState) => visualState[state]

export function isInterruptionRelease(
  previous: VisualState | null,
  next: VisualState,
) {
  return previous === 'speaking' && next === 'listening'
}

type InterruptionReleaseTimers = {
  setTimeout(callback: () => void, delayMs: number): unknown
  clearTimeout(handle: unknown): void
}

const interruptionReleaseTimers: InterruptionReleaseTimers = {
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) => {
    globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>)
  },
}

export function createInterruptionReleaseLatch(
  onActive: (active: boolean) => void,
  timers: InterruptionReleaseTimers = interruptionReleaseTimers,
) {
  let active = false
  let timer: unknown = null

  const update = (previous: VisualState | null, next: VisualState) => {
    if (!isInterruptionRelease(previous, next)) return
    if (timer !== null) timers.clearTimeout(timer)
    if (!active) {
      active = true
      onActive(true)
    }
    timer = timers.setTimeout(() => {
      timer = null
      if (!active) return
      active = false
      onActive(false)
    }, MOTION_TIMING.interruptMs)
  }

  return {
    current: () => active,
    dispose() {
      if (timer !== null) timers.clearTimeout(timer)
      timer = null
      active = false
    },
    update,
  }
}

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
  if (forceLow || (current === 'high' && fps < 45 && stableForMs >= 2000)) {
    return 'low'
  }
  if (current === 'low' && fps >= 58 && stableForMs >= 5000) return 'high'
  return current
}
