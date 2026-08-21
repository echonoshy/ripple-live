import { useEffect, useRef, type RefObject } from 'react'

export const EDGE_SWIPE_START_ZONE = 28
export const EDGE_SWIPE_COMMIT_DISTANCE = 72

const DIRECTION_LOCK_DISTANCE = 10
const MAX_VISUAL_OFFSET = 118
const SETTLE_DURATION_MS = 140

export function canStartEdgeSwipe(clientX: number, button: number) {
  return button === 0 && clientX >= 0 && clientX <= EDGE_SWIPE_START_ZONE
}

export function isHorizontalBackIntent(deltaX: number, deltaY: number) {
  return deltaX >= DIRECTION_LOCK_DISTANCE && deltaX > Math.abs(deltaY) * 1.15
}

export function edgeSwipeOffset(deltaX: number) {
  if (deltaX <= 0) return 0
  return Math.min(MAX_VISUAL_OFFSET, deltaX * 0.72)
}

export function shouldCommitEdgeSwipe(deltaX: number) {
  return deltaX >= EDGE_SWIPE_COMMIT_DISTANCE
}

type ActiveSwipe = {
  pointerId: number
  startX: number
  startY: number
  dragging: boolean
}

export function useEdgeSwipeBack({
  rootRef,
  enabled,
  onBack,
}: {
  rootRef: RefObject<HTMLElement | null>
  enabled: boolean
  onBack(): void
}) {
  const onBackRef = useRef(onBack)

  useEffect(() => {
    onBackRef.current = onBack
  }, [onBack])

  useEffect(() => {
    const root = rootRef.current
    if (!root || !enabled) return

    let active: ActiveSwipe | null = null
    let settleTimer: number | null = null

    const clearVisualState = () => {
      root.removeAttribute('data-edge-swipe-state')
      root.style.removeProperty('--edge-swipe-offset')
      root.style.removeProperty('--edge-swipe-progress')
    }

    const cancelSwipe = () => {
      active = null
      clearVisualState()
    }

    const onPointerDown = (event: PointerEvent) => {
      if (!event.isPrimary || !canStartEdgeSwipe(event.clientX, event.button)) return
      const target = event.target as HTMLElement | null
      if (target?.closest('input, textarea, select, [contenteditable="true"], [data-edge-swipe-ignore]')) return
      active = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        dragging: false,
      }
    }

    const onPointerMove = (event: PointerEvent) => {
      if (!active || event.pointerId !== active.pointerId) return
      const deltaX = event.clientX - active.startX
      const deltaY = event.clientY - active.startY

      if (!active.dragging) {
        if (deltaX < 0 || Math.abs(deltaY) > Math.max(12, deltaX)) {
          cancelSwipe()
          return
        }
        if (!isHorizontalBackIntent(deltaX, deltaY)) return
        active.dragging = true
        root.setAttribute('data-edge-swipe-state', 'dragging')
      }

      event.preventDefault()
      event.stopPropagation()
      const offset = edgeSwipeOffset(deltaX)
      root.style.setProperty('--edge-swipe-offset', `${offset}px`)
      root.style.setProperty(
        '--edge-swipe-progress',
        String(Math.min(1, deltaX / EDGE_SWIPE_COMMIT_DISTANCE)),
      )
    }

    const finishSwipe = (event: PointerEvent) => {
      if (!active || event.pointerId !== active.pointerId) return
      const swipe = active
      active = null
      if (!swipe.dragging) {
        clearVisualState()
        return
      }

      event.preventDefault()
      event.stopPropagation()
      const deltaX = event.clientX - swipe.startX
      const commits = shouldCommitEdgeSwipe(deltaX)
      root.setAttribute('data-edge-swipe-state', 'settling')
      root.style.setProperty('--edge-swipe-offset', commits ? `${MAX_VISUAL_OFFSET}px` : '0px')
      root.style.setProperty('--edge-swipe-progress', commits ? '1' : '0')

      settleTimer = window.setTimeout(() => {
        clearVisualState()
        if (commits) onBackRef.current()
      }, window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : SETTLE_DURATION_MS)
    }

    root.addEventListener('pointerdown', onPointerDown, { capture: true })
    root.addEventListener('pointermove', onPointerMove, { capture: true, passive: false })
    root.addEventListener('pointerup', finishSwipe, { capture: true, passive: false })
    root.addEventListener('pointercancel', cancelSwipe, { capture: true })

    return () => {
      if (settleTimer !== null) window.clearTimeout(settleTimer)
      clearVisualState()
      root.removeEventListener('pointerdown', onPointerDown, { capture: true })
      root.removeEventListener('pointermove', onPointerMove, { capture: true })
      root.removeEventListener('pointerup', finishSwipe, { capture: true })
      root.removeEventListener('pointercancel', cancelSwipe, { capture: true })
    }
  }, [enabled, rootRef])
}
