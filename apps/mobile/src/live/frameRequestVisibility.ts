type VisibilityTimers = {
  now(): number
  setTimeout(callback: () => void, delayMs: number): unknown
  clearTimeout(handle: unknown): void
}

type MinimumVisibleSignalOptions = {
  minimumMs: number
  onVisible(visible: boolean): void
  timers: VisibilityTimers
}

/** Keeps a short-lived realtime signal visible across at least one paint. */
export function createMinimumVisibleSignal({
  minimumMs,
  onVisible,
  timers,
}: MinimumVisibleSignalOptions) {
  let disposed = false
  let visible = false
  let shownAt = 0
  let hideTimer: unknown

  const cancelHide = () => {
    if (hideTimer === undefined) return
    timers.clearTimeout(hideTimer)
    hideTimer = undefined
  }
  const setVisible = (nextVisible: boolean) => {
    if (visible === nextVisible) return
    visible = nextVisible
    onVisible(nextVisible)
  }

  return {
    dispose() {
      if (disposed) return
      disposed = true
      cancelHide()
      setVisible(false)
    },
    update(active: boolean) {
      if (disposed) return
      if (active) {
        cancelHide()
        shownAt = timers.now()
        setVisible(true)
        return
      }
      if (!visible || hideTimer !== undefined) return
      const remaining = Math.max(0, minimumMs - (timers.now() - shownAt))
      hideTimer = timers.setTimeout(() => {
        hideTimer = undefined
        if (!disposed) setVisible(false)
      }, remaining)
    },
  }
}
