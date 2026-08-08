import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  isInterruptionRelease,
  type QualityTier,
  type VisualState,
} from '../live/motion'
import {
  createOrbRenderer,
  type OrbRenderer,
} from '../live/orbRenderer'
import { startOrbLifecycle } from '../live/orbLifecycle'
import type { RippleSignal } from '../live/ripple'
import '../live/LiveCall.css'

export type LiveOrbProps = {
  state: VisualState
  inputLevel: number
  outputLevel: number
  rippleSignal?: RippleSignal | null
}

export function LiveOrb(props: LiveOrbProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const previousVisualStateRef = useRef<VisualState | null>(null)
  const [fallback, setFallback] = useState(false)
  const latestProps = useRef({
    state: props.state,
    inputLevel: props.inputLevel,
    outputLevel: props.outputLevel,
    reducedMotion: false,
    qualityTier: 'high' as QualityTier,
    rippleSignal: props.rippleSignal ?? null,
  })
  latestProps.current.state = props.state
  latestProps.current.inputLevel = props.inputLevel
  latestProps.current.outputLevel = props.outputLevel
  latestProps.current.rippleSignal = props.rippleSignal ?? null

  const previousState = previousVisualStateRef.current
  const interruptionRelease = isInterruptionRelease(previousState, props.state)

  useLayoutEffect(() => {
    previousVisualStateRef.current = props.state
  }, [props.state])

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

    return startOrbLifecycle(
      renderer,
      canvas,
      latestProps,
      () => setFallback(true),
    )
  }, [])

  const stateClass = `is-${props.state}`
  const interruptionClass = interruptionRelease ? ' is-interruption-release' : ''
  return fallback
    ? <div className={`live-orb-fallback ${stateClass}${interruptionClass}`} aria-hidden="true" />
    : <canvas ref={canvasRef} className={`live-orb-canvas ${stateClass}${interruptionClass}`} aria-hidden="true" />
}
