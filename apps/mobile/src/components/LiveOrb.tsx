import { useEffect, useRef, useState } from 'react'
import type { QualityTier, VisualState } from '../live/motion'
import {
  createOrbRenderer,
  type OrbRenderer,
} from '../live/orbRenderer'
import { startOrbLifecycle } from '../live/orbLifecycle'
import '../live/LiveCall.css'

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
  return fallback
    ? <div className={`live-orb-fallback ${stateClass}`} aria-hidden="true" />
    : <canvas ref={canvasRef} className={`live-orb-canvas ${stateClass}`} aria-hidden="true" />
}
