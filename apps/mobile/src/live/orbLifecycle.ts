import type { OrbRenderer } from './orbRenderer'
import type { QualityTier, VisualState } from './motion'
import { nextQualityTier, smoothLevel } from './motion'
import {
  advanceRippleSignals,
  createRippleState,
  type RippleSignal,
  type RippleSignalId,
} from './ripple'

export type OrbLifecycleState = {
  current: {
    state: VisualState
    inputLevel: number
    outputLevel: number
    reducedMotion: boolean
    qualityTier: QualityTier
    rippleSignal?: RippleSignal | null
    rippleSignals?: readonly RippleSignal[]
    onRippleSignalsConsumed?(signalId: RippleSignalId): void
  }
}

type BatteryStatus = EventTarget & {
  charging: boolean
  level: number
}

type NavigatorWithBattery = Navigator & {
  getBattery?: () => Promise<BatteryStatus>
}

const pixelRatioFor = (qualityTier: QualityTier) => {
  const ratio = Number.isFinite(window.devicePixelRatio)
    ? window.devicePixelRatio
    : 1
  return qualityTier === 'high'
    ? Math.min(Math.max(ratio, 1), 2)
    : Math.min(Math.max(ratio, 1), 1.25)
}

const frameIntervalFor = (qualityTier: QualityTier) => (
  1000 / (qualityTier === 'high' ? 60 : 30)
)
const MAX_FRAME_GAP_MS = 1000
const PACING_EPSILON_MS = 0.01
const LEVEL_FRAME_MS = 1000 / 60
const LEVEL_ATTACK_ALPHA = 0.085
const LEVEL_RELEASE_ALPHA = 0.035

const levelAlphaForFrame = (baseAlpha: number, elapsedMs: number) => (
  1 - Math.pow(1 - baseAlpha, Math.max(0.25, Math.min(4, elapsedMs / LEVEL_FRAME_MS)))
)

export function startOrbLifecycle(
  renderer: OrbRenderer,
  canvas: HTMLCanvasElement,
  latestProps: OrbLifecycleState,
  onFallback: () => void,
) {
  let active = true
  let cleaned = false
  let fallbackEntered = false
  let frame: number | null = null
  let battery: BatteryStatus | null = null
  let batteryLevelListening = false
  let batteryChargingListening = false
  let batteryLow = false
  let forcedLow = false
  let motionQuery: MediaQueryList | null = null
  let motionListening = false
  let observer: ResizeObserver | null = null
  const frameTimes: number[] = []
  let observationStartedAt = performance.now()
  let conditionStartedAt = observationStartedAt
  let lastCondition: 'slow' | 'stable' | 'neutral' = 'neutral'
  let lastWidth = 0
  let lastHeight = 0
  let pacedQuality: QualityTier | null = null
  let lastRenderedAt: number | null = null
  let nextRenderDeadline: number | null = null
  let lastAnimationFrameAt: number | null = null
  let rippleState = createRippleState()
  let smoothedInputLevel = 0
  let smoothedOutputLevel = 0
  let lastLevelUpdateAt: number | null = null

  const smoothVisualLevel = (previous: number, target: number, elapsedMs: number) => {
    const alpha = levelAlphaForFrame(
      target > previous ? LEVEL_ATTACK_ALPHA : LEVEL_RELEASE_ALPHA,
      elapsedMs,
    )
    return smoothLevel(previous, target, alpha)
  }

  const visualLevelsAt = (nowMs: number) => {
    const elapsedMs = lastLevelUpdateAt === null
      ? LEVEL_FRAME_MS
      : Math.max(0, nowMs - lastLevelUpdateAt)
    lastLevelUpdateAt = nowMs
    smoothedInputLevel = smoothVisualLevel(
      smoothedInputLevel,
      latestProps.current.inputLevel,
      elapsedMs,
    )
    smoothedOutputLevel = smoothVisualLevel(
      smoothedOutputLevel,
      latestProps.current.outputLevel,
      elapsedMs,
    )
    return {
      inputLevel: smoothedInputLevel,
      outputLevel: smoothedOutputLevel,
    }
  }

  const safely = (operation: () => void) => {
    try {
      operation()
    } catch {
      // Teardown is best-effort so one browser API cannot leak the others.
    }
  }
  const cleanup = () => {
    if (cleaned) return
    cleaned = true
    active = false
    if (frame !== null) safely(() => cancelAnimationFrame(frame!))
    if (observer) safely(() => observer?.disconnect())
    if (motionQuery && motionListening) {
      safely(() => motionQuery?.removeEventListener('change', updateReducedMotion))
    }
    if (battery && batteryLevelListening) {
      safely(() => battery?.removeEventListener('levelchange', updateBattery))
    }
    if (battery && batteryChargingListening) {
      safely(() => battery?.removeEventListener('chargingchange', updateBattery))
    }
    safely(() => renderer.dispose())
  }
  const enterFallback = () => {
    if (fallbackEntered || cleaned) return
    fallbackEntered = true
    cleanup()
    safely(onFallback)
  }
  const resize = () => {
    renderer.resize(
      lastWidth,
      lastHeight,
      pixelRatioFor(latestProps.current.qualityTier),
    )
  }
  const resetQualityObservation = (nowMs: number) => {
    frameTimes.length = 0
    observationStartedAt = nowMs
    conditionStartedAt = nowMs
    lastCondition = 'neutral'
  }
  const applyPowerPolicy = () => {
    const nextForcedLow = latestProps.current.reducedMotion || batteryLow
    if (nextForcedLow !== forcedLow) {
      forcedLow = nextForcedLow
      resetQualityObservation(performance.now())
    }
    const next = nextQualityTier(
      latestProps.current.qualityTier,
      60,
      0,
      forcedLow,
    )
    if (next !== latestProps.current.qualityTier) {
      latestProps.current.qualityTier = next
      resize()
    }
  }
  const updateReducedMotion = () => {
    if (!motionQuery) return
    latestProps.current.reducedMotion = motionQuery.matches
    applyPowerPolicy()
  }
  const updateBattery = () => {
    if (!battery) return
    batteryLow = battery.level <= 0.15 && !battery.charging
    applyPowerPolicy()
  }
  const resetFramePacing = (nowMs: number) => {
    const qualityTier = latestProps.current.qualityTier
    pacedQuality = qualityTier
    lastRenderedAt = nowMs
    nextRenderDeadline = nowMs + frameIntervalFor(qualityTier)
  }
  const resetAfterFrameGap = (nowMs: number) => {
    const hadGap = lastAnimationFrameAt !== null && (
      nowMs < lastAnimationFrameAt
      || nowMs - lastAnimationFrameAt > MAX_FRAME_GAP_MS
    )
    lastAnimationFrameAt = nowMs
    if (!hadGap) return false
    resetQualityObservation(nowMs)
    resetFramePacing(nowMs)
    return true
  }
  const shouldRender = (nowMs: number) => {
    const qualityTier = latestProps.current.qualityTier
    const interval = frameIntervalFor(qualityTier)
    if (lastRenderedAt === null || !Number.isFinite(nowMs)) {
      resetFramePacing(nowMs)
      return true
    }
    if (pacedQuality !== qualityTier) {
      pacedQuality = qualityTier
      nextRenderDeadline = lastRenderedAt + interval
    }
    if (nextRenderDeadline === null || nowMs < lastRenderedAt) {
      resetFramePacing(nowMs)
      return true
    }
    if (nowMs + PACING_EPSILON_MS < nextRenderDeadline) return false

    const elapsedIntervals = Math.floor(
      (nowMs - nextRenderDeadline + PACING_EPSILON_MS) / interval,
    ) + 1
    nextRenderDeadline += elapsedIntervals * interval
    lastRenderedAt = nowMs
    return true
  }

  const draw = (nowMs: number) => {
    if (!active) return
    try {
      const resumed = resetAfterFrameGap(nowMs)
      frameTimes.push(nowMs)
      const windowStart = nowMs - 2000
      while (frameTimes.length > 1 && frameTimes[0] < windowStart) {
        frameTimes.shift()
      }
      if (nowMs - observationStartedAt >= 2000 && frameTimes.length > 1) {
        const measuredDuration = nowMs - frameTimes[0]
        const fps = (frameTimes.length - 1) * 1000 / Math.max(measuredDuration, 1)
        const condition = fps < 45
          ? 'slow'
          : fps >= 58
            ? 'stable'
            : 'neutral'
        if (condition !== lastCondition) {
          lastCondition = condition
          conditionStartedAt = nowMs - measuredDuration
        }
        const stableForMs = condition === 'neutral'
          ? 0
          : nowMs - conditionStartedAt
        const next = nextQualityTier(
          latestProps.current.qualityTier,
          fps,
          stableForMs,
          forcedLow,
        )
        if (next !== latestProps.current.qualityTier) {
          latestProps.current.qualityTier = next
          resize()
        }
      }

      if (resumed || shouldRender(nowMs)) {
        const visualLevels = visualLevelsAt(nowMs)
        const priorConsumedSignalId = rippleState.lastConsumedSignalId
        const pendingSignals = latestProps.current.rippleSignals
          ?? (latestProps.current.rippleSignal
            ? [latestProps.current.rippleSignal]
            : [])
        const ripple = advanceRippleSignals(rippleState, {
          signals: pendingSignals,
          visualState: latestProps.current.state,
          outputLevel: latestProps.current.outputLevel,
          reducedMotion: latestProps.current.reducedMotion,
        }, nowMs)
        rippleState = ripple.state
        renderer.update({
          ...latestProps.current,
          ...visualLevels,
          nowMs,
          rippleProgress: ripple.frame.progress,
          rippleAlpha: ripple.frame.alpha,
          haloPulse: ripple.frame.haloPulse,
        })
        if (
          ripple.state.lastConsumedSignalId !== null
          && ripple.state.lastConsumedSignalId !== priorConsumedSignalId
        ) {
          latestProps.current.onRippleSignalsConsumed?.(
            ripple.state.lastConsumedSignalId,
          )
        }
      }
      frame = requestAnimationFrame(draw)
    } catch {
      enterFallback()
    }
  }

  try {
    motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
    updateReducedMotion()
    motionQuery.addEventListener('change', updateReducedMotion)
    motionListening = true

    const batteryNavigator = navigator as NavigatorWithBattery
    if (batteryNavigator.getBattery) {
      void batteryNavigator.getBattery().then((status) => {
        if (!active) return
        battery = status
        try {
          updateBattery()
          battery.addEventListener('levelchange', updateBattery)
          batteryLevelListening = true
          battery.addEventListener('chargingchange', updateBattery)
          batteryChargingListening = true
        } catch {
          enterFallback()
        }
      }).catch(() => {
        // Battery status is an optional hint; rendering does not depend on it.
      })
    }

    observer = new ResizeObserver(([entry]) => {
      if (!entry || !active) return
      try {
        lastWidth = Math.max(0, entry.contentRect.width)
        lastHeight = Math.max(0, entry.contentRect.height)
        resize()
      } catch {
        enterFallback()
      }
    })
    observer.observe(canvas)
    const initialRect = canvas.getBoundingClientRect()
    lastWidth = Math.max(0, initialRect.width)
    lastHeight = Math.max(0, initialRect.height)
    resize()
    frame = requestAnimationFrame(draw)
  } catch {
    enterFallback()
  }

  return cleanup
}
