import type { VisualState } from './motion'

export type RippleKind = 'speech' | 'assistant' | 'tool' | 'interrupt'
declare const rippleSignalIdBrand: unique symbol
export type RippleSignalId = number & { readonly [rippleSignalIdBrand]: true }
export type RippleSignal = { readonly id: RippleSignalId; readonly kind: RippleKind }
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

const HALO_PULSE_MS = 160
const ASSISTANT_EMPHASIS_LEVEL = 0.28
let latestRippleSignalId = 0

export function nextRippleSignalId(lastId: number): number {
  if (!Number.isSafeInteger(lastId) || lastId < 0 || lastId >= Number.MAX_SAFE_INTEGER) {
    throw new RangeError('Ripple signal ID exceeds the safe integer range')
  }
  return lastId + 1
}

export function createRippleSignal(kind: RippleKind): RippleSignal {
  latestRippleSignalId = nextRippleSignalId(latestRippleSignalId)
  return { id: latestRippleSignalId as RippleSignalId, kind }
}

export type RippleState = {
  activeKind: RippleKind | null
  activeStartedAtMs: number | null
  assistantEmphasisUsed: boolean
  cooldownUntilMs: number
  haloPulseStartedAtMs: number | null
  lastConsumedSignalId: RippleSignalId | null
  previousVisualState: VisualState | null
}

export function createRippleState(): RippleState {
  return {
    activeKind: null,
    activeStartedAtMs: null,
    assistantEmphasisUsed: false,
    cooldownUntilMs: 0,
    haloPulseStartedAtMs: null,
    lastConsumedSignalId: null,
    previousVisualState: null,
  }
}

const hasActiveRing = (state: RippleState, nowMs: number) => (
  state.activeKind !== null
  && state.activeStartedAtMs !== null
  && nowMs - state.activeStartedAtMs < RIPPLE_MOTION.durationMs
)

const haloPulseAt = (startedAtMs: number | null, nowMs: number) => {
  if (startedAtMs === null) return 0
  return Math.max(0, 1 - (nowMs - startedAtMs) / HALO_PULSE_MS)
}

export function advanceRipple(
  state: RippleState,
  input: RippleInput,
  nowMs: number,
): { state: RippleState; frame: RippleFrame } {
  const active = !input.reducedMotion && hasActiveRing(state, nowMs)
  let next: RippleState = {
    ...state,
    activeKind: active ? state.activeKind : null,
    activeStartedAtMs: active ? state.activeStartedAtMs : null,
    previousVisualState: input.visualState,
  }

  if (state.previousVisualState === 'speaking' && input.visualState !== 'speaking') {
    next = { ...next, assistantEmphasisUsed: false }
  }

  let requestedKind: RippleKind | null = null
  if (
    input.signal !== null
    && (state.lastConsumedSignalId === null || input.signal.id > state.lastConsumedSignalId)
  ) {
    requestedKind = input.signal.kind
    next = { ...next, lastConsumedSignalId: input.signal.id }
  }

  if (
    input.visualState === 'speaking'
    && !next.assistantEmphasisUsed
    && input.outputLevel >= ASSISTANT_EMPHASIS_LEVEL
  ) {
    requestedKind = requestedKind ?? 'assistant'
    next = { ...next, assistantEmphasisUsed: true }
  }

  if (requestedKind !== null) {
    const canStart = !input.reducedMotion && !hasActiveRing(next, nowMs) && nowMs >= next.cooldownUntilMs
    if (canStart) {
      next = {
        ...next,
        activeKind: requestedKind,
        activeStartedAtMs: nowMs,
        cooldownUntilMs: nowMs + RIPPLE_MOTION.cooldownMs,
      }
    } else {
      next = { ...next, haloPulseStartedAtMs: nowMs }
    }
  }

  if (next.activeKind === null || next.activeStartedAtMs === null) {
    return {
      state: next,
      frame: { kind: null, progress: null, alpha: 0, haloPulse: haloPulseAt(next.haloPulseStartedAtMs, nowMs) },
    }
  }

  const progress = Math.min(1, Math.max(0, (nowMs - next.activeStartedAtMs) / RIPPLE_MOTION.durationMs))
  return {
    state: next,
    frame: {
      kind: next.activeKind,
      progress,
      alpha: RIPPLE_MOTION.maximumAlpha * (1 - progress) ** 2,
      haloPulse: haloPulseAt(next.haloPulseStartedAtMs, nowMs),
    },
  }
}
