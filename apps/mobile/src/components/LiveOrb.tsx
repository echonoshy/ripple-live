import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  createInterruptionReleaseLatch,
  isInterruptionRelease,
  type QualityTier,
  type VisualState,
} from '../live/motion'
import type { OrbRenderer } from '../live/orbRenderer'
import { createPetRenderer } from '../live/petRenderer'
import { startOrbLifecycle } from '../live/orbLifecycle'
import type { RippleSignal, RippleSignalId } from '../live/ripple'
import starryAvatarUrl from '../assets/starry-avatar.webp'
import starryAvatarHdUrl from '../assets/starry-avatar-states@2x.png'
import starryAvatarFailedGifUrl from '../assets/pet-gifs/starry-avatar-failed.gif'
import starryAvatarIdleGifUrl from '../assets/pet-gifs/starry-avatar-idle.gif'
import starryAvatarReviewGifUrl from '../assets/pet-gifs/starry-avatar-review.gif'
import starryAvatarRunningGifUrl from '../assets/pet-gifs/starry-avatar-running.gif'
import starryAvatarWaitingGifUrl from '../assets/pet-gifs/starry-avatar-waiting.gif'
import starryAvatarWavingGifUrl from '../assets/pet-gifs/starry-avatar-waving.gif'
import '../live/LiveCall.css'

const starryAvatarGifUrls = {
  idle: starryAvatarIdleGifUrl,
  waving: starryAvatarWavingGifUrl,
  failed: starryAvatarFailedGifUrl,
  waiting: starryAvatarWaitingGifUrl,
  running: starryAvatarRunningGifUrl,
  review: starryAvatarReviewGifUrl,
} as const

export type LiveOrbProps = {
  state: VisualState
  inputLevel: number
  outputLevel: number
  rippleSignals?: readonly RippleSignal[]
  onRippleSignalsConsumed?(signalId: RippleSignalId): void
}

export function LiveOrb(props: LiveOrbProps) {
  const queuedRippleSignals = props.rippleSignals
  const acknowledgeRippleSignals = props.onRippleSignalsConsumed
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const previousVisualStateRef = useRef<VisualState | null>(null)
  const [fallback, setFallback] = useState(false)
  const [interruptionReleaseHeld, setInterruptionReleaseHeld] = useState(false)
  const interruptionReleaseLatchRef = useRef<ReturnType<
    typeof createInterruptionReleaseLatch
  > | null>(null)
  if (!interruptionReleaseLatchRef.current) {
    interruptionReleaseLatchRef.current = createInterruptionReleaseLatch(
      setInterruptionReleaseHeld,
    )
  }
  const latestProps = useRef({
    state: props.state,
    inputLevel: props.inputLevel,
    outputLevel: props.outputLevel,
    reducedMotion: false,
    qualityTier: 'high' as QualityTier,
    rippleSignals: props.rippleSignals ?? [],
    onRippleSignalsConsumed: props.onRippleSignalsConsumed,
  })
  latestProps.current.state = props.state
  latestProps.current.inputLevel = props.inputLevel
  latestProps.current.outputLevel = props.outputLevel
  latestProps.current.rippleSignals = props.rippleSignals ?? []
  latestProps.current.onRippleSignalsConsumed = props.onRippleSignalsConsumed

  const previousState = previousVisualStateRef.current
  const interruptionReleaseEntering = isInterruptionRelease(previousState, props.state)

  useLayoutEffect(() => {
    interruptionReleaseLatchRef.current?.update(
      previousVisualStateRef.current,
      props.state,
    )
    previousVisualStateRef.current = props.state
  }, [props.state])

  useEffect(() => () => {
    interruptionReleaseLatchRef.current?.dispose()
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    let renderer: OrbRenderer
    try {
      renderer = createPetRenderer(
        canvas,
        starryAvatarUrl,
        starryAvatarHdUrl,
        starryAvatarGifUrls,
      )
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

  useEffect(() => {
    if (!fallback || !queuedRippleSignals?.length) return
    acknowledgeRippleSignals?.(
      queuedRippleSignals[queuedRippleSignals.length - 1].id,
    )
  }, [acknowledgeRippleSignals, fallback, queuedRippleSignals])

  const stateClass = `is-${props.state}`
  const interruptionRelease = interruptionReleaseEntering || interruptionReleaseHeld
  const interruptionClass = interruptionRelease ? ' is-interruption-release' : ''
  return fallback
    ? <div className={`live-orb-fallback ${stateClass}${interruptionClass}`} aria-hidden="true" />
    : <canvas ref={canvasRef} className={`live-orb-canvas ${stateClass}${interruptionClass}`} aria-hidden="true" />
}
