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
  captionHoldMs: 1800,
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
