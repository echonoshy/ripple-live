import type { OrbRenderer } from './orbRenderer'
import type { QualityTier, VisualState } from './motion'
import { nextQualityTier } from './motion'

export type OrbLifecycleState = {
  current: {
    state: VisualState
    inputLevel: number
    outputLevel: number
    reducedMotion: boolean
    qualityTier: QualityTier
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

const LOW_QUALITY_FRAME_INTERVAL_MS = 1000 / 30

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
  let lastRenderedAt = Number.NEGATIVE_INFINITY

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

  const draw = (nowMs: number) => {
    try {
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

      if (
        latestProps.current.qualityTier === 'high'
        || nowMs - lastRenderedAt >= LOW_QUALITY_FRAME_INTERVAL_MS
      ) {
        renderer.update({ ...latestProps.current, nowMs })
        lastRenderedAt = nowMs
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
