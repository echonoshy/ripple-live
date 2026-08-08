import { useEffect, useRef, useState } from 'react'
import type { QualityTier, VisualState } from '../live/motion'
import { nextQualityTier } from '../live/motion'
import {
  createOrbRenderer,
  type OrbRenderer,
} from '../live/orbRenderer'
import '../live/LiveCall.css'

export type LiveOrbProps = {
  state: VisualState
  inputLevel: number
  outputLevel: number
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
    const canvas = canvasRef.current
    if (!canvas) return

    let renderer: OrbRenderer
    try {
      renderer = createOrbRenderer(canvas)
    } catch {
      setFallback(true)
      return
    }

    let active = true
    let frame = 0
    let battery: BatteryStatus | null = null
    let batteryLow = false
    let forcedLow = false
    const frameTimes: number[] = []
    let observationStartedAt = performance.now()
    let conditionStartedAt = observationStartedAt
    let lastCondition: 'slow' | 'stable' | 'neutral' = 'neutral'
    let lastWidth = 0
    let lastHeight = 0

    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
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
      latestProps.current.reducedMotion = motionQuery.matches
      applyPowerPolicy()
    }
    const updateBattery = () => {
      if (!battery) return
      batteryLow = battery.level <= 0.15 && !battery.charging
      applyPowerPolicy()
    }

    updateReducedMotion()
    motionQuery.addEventListener('change', updateReducedMotion)

    const batteryNavigator = navigator as NavigatorWithBattery
    if (batteryNavigator.getBattery) {
      void batteryNavigator.getBattery().then((status) => {
        if (!active) return
        battery = status
        updateBattery()
        battery.addEventListener('levelchange', updateBattery)
        battery.addEventListener('chargingchange', updateBattery)
      }).catch(() => {
        // Battery status is an optional hint; rendering does not depend on it.
      })
    }

    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return
      lastWidth = Math.max(0, entry.contentRect.width)
      lastHeight = Math.max(0, entry.contentRect.height)
      resize()
    })
    observer.observe(canvas)
    const initialRect = canvas.getBoundingClientRect()
    lastWidth = Math.max(0, initialRect.width)
    lastHeight = Math.max(0, initialRect.height)
    resize()

    const draw = (nowMs: number) => {
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

      renderer.update({ ...latestProps.current, nowMs })
      frame = requestAnimationFrame(draw)
    }
    frame = requestAnimationFrame(draw)

    return () => {
      active = false
      cancelAnimationFrame(frame)
      observer.disconnect()
      motionQuery.removeEventListener('change', updateReducedMotion)
      battery?.removeEventListener('levelchange', updateBattery)
      battery?.removeEventListener('chargingchange', updateBattery)
      renderer.dispose()
    }
  }, [])

  const stateClass = `is-${props.state}`
  return fallback
    ? <div className={`live-orb-fallback ${stateClass}`} aria-hidden="true" />
    : <canvas ref={canvasRef} className={`live-orb-canvas ${stateClass}`} aria-hidden="true" />
}
