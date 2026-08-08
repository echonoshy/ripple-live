import assert from 'node:assert/strict'
import test from 'node:test'
import {
  startOrbLifecycle,
  type OrbLifecycleState,
} from '../src/live/orbLifecycle.ts'
import type { OrbRenderer } from '../src/live/orbRenderer.ts'

type GlobalKey =
  | 'window'
  | 'navigator'
  | 'performance'
  | 'ResizeObserver'
  | 'requestAnimationFrame'
  | 'cancelAnimationFrame'

function installBrowserFakes(
  observeThrows: boolean,
  options: { reducedMotion?: boolean; captureFrames?: boolean; nowMs?: number } = {},
) {
  const descriptors = new Map<GlobalKey, PropertyDescriptor | undefined>()
  const setGlobal = (key: GlobalKey, value: unknown) => {
    descriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
    Object.defineProperty(globalThis, key, {
      configurable: true,
      writable: true,
      value,
    })
  }

  const calls = {
    mediaAdded: 0,
    mediaRemoved: 0,
    observerDisconnected: 0,
    framesCancelled: [] as number[],
    frames: [] as FrameRequestCallback[],
  }
  let nowMs = options.nowMs ?? 0
  const motionListeners = new Set<() => void>()
  const mediaQuery = {
    matches: options.reducedMotion ?? false,
    addEventListener: (_type: string, listener: () => void) => {
      calls.mediaAdded += 1
      motionListeners.add(listener)
    },
    removeEventListener: (_type: string, listener: () => void) => {
      calls.mediaRemoved += 1
      motionListeners.delete(listener)
    },
  }

  class FakeResizeObserver {
    observe() {
      if (observeThrows) throw new Error('observe_failed')
    }
    disconnect() { calls.observerDisconnected += 1 }
  }

  setGlobal('window', { devicePixelRatio: 2, matchMedia: () => mediaQuery })
  setGlobal('navigator', {})
  setGlobal('performance', { now: () => nowMs })
  setGlobal('ResizeObserver', FakeResizeObserver)
  setGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    if (options.captureFrames) calls.frames.push(callback)
    return 41 + calls.frames.length
  })
  setGlobal('cancelAnimationFrame', (frame: number) => {
    calls.framesCancelled.push(frame)
  })

  return {
    calls,
    setNow(nextNowMs: number) { nowMs = nextNowMs },
    setReducedMotion(matches: boolean) {
      mediaQuery.matches = matches
      for (const listener of motionListeners) listener()
    },
    restore() {
      for (const [key, descriptor] of descriptors) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor)
        else Reflect.deleteProperty(globalThis, key)
      }
    },
  }
}

function createHarness() {
  let disposals = 0
  let fallbacks = 0
  const renderer: OrbRenderer = {
    update() {},
    resize() {},
    dispose() { disposals += 1 },
  }
  const canvas = {
    getBoundingClientRect: () => ({ width: 120, height: 120 }),
  } as unknown as HTMLCanvasElement
  const latestProps: OrbLifecycleState = {
    current: {
      state: 'listening',
      inputLevel: 0,
      outputLevel: 0,
      reducedMotion: false,
      qualityTier: 'high',
    },
  }
  return {
    canvas,
    latestProps,
    renderer,
    counts: {
      get disposals() { return disposals },
      get fallbacks() { return fallbacks },
    },
    onFallback: () => { fallbacks += 1 },
  }
}

function runNextFrame(
  browser: ReturnType<typeof installBrowserFakes>,
  nowMs: number,
) {
  const callback = browser.calls.frames.shift()
  assert.ok(callback, `expected a queued frame at ${nowMs}ms`)
  callback(nowMs)
}

function runCadence(
  browser: ReturnType<typeof installBrowserFakes>,
  fromMs: number,
  throughMs: number,
  fps: number,
) {
  const interval = 1000 / fps
  for (let nowMs = fromMs; nowMs <= throughMs + 0.01; nowMs += interval) {
    runNextFrame(browser, nowMs)
  }
}

test('partial observer setup failure releases resources and enters fallback', () => {
  const browser = installBrowserFakes(true)
  const harness = createHarness()
  let cleanup = () => {}

  try {
    assert.doesNotThrow(() => {
      cleanup = startOrbLifecycle(
        harness.renderer,
        harness.canvas,
        harness.latestProps,
        harness.onFallback,
      )
    })
    assert.equal(harness.counts.fallbacks, 1)
    assert.equal(harness.counts.disposals, 1)
    assert.equal(browser.calls.mediaAdded, 1)
    assert.equal(browser.calls.mediaRemoved, 1)
    assert.equal(browser.calls.observerDisconnected, 1)

    cleanup()
    assert.equal(harness.counts.disposals, 1)
  } finally {
    browser.restore()
  }
})

test('successful lifecycle cleanup is idempotent', () => {
  const browser = installBrowserFakes(false)
  const harness = createHarness()

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    cleanup()
    cleanup()

    assert.equal(harness.counts.fallbacks, 0)
    assert.equal(harness.counts.disposals, 1)
    assert.deepEqual(browser.calls.framesCancelled, [41])
    assert.equal(browser.calls.mediaRemoved, 1)
    assert.equal(browser.calls.observerDisconnected, 1)
  } finally {
    browser.restore()
  }
})

test('high-quality mode targets at most 60fps on 120Hz callbacks', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()
  const updates: number[] = []
  harness.renderer.update = (frame) => updates.push(frame.nowMs)

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    const callbacks = Array.from({ length: 13 }, (_, index) => index * (1000 / 120))
    for (const nowMs of callbacks) runNextFrame(browser, nowMs)

    assert.equal(updates[0], 0)
    assert.deepEqual(updates, callbacks.filter((_, index) => index % 2 === 0))
    cleanup()
  } finally {
    browser.restore()
  }
})

test('low-quality mode keeps deadline cadence across jittered 30fps boundaries', () => {
  const browser = installBrowserFakes(false, {
    captureFrames: true,
    reducedMotion: true,
  })
  const harness = createHarness()
  const updates: number[] = []
  harness.renderer.update = (frame) => updates.push(frame.nowMs)

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    for (const nowMs of [0, 16, 33.2, 33.4, 49.5, 66.5, 66.8, 83, 99.9, 100.1, 116, 133.4]) {
      runNextFrame(browser, nowMs)
    }

    assert.equal(harness.latestProps.current.qualityTier, 'low')
    assert.deepEqual(updates, [0, 33.4, 66.8, 100.1, 133.4])
    cleanup()
  } finally {
    browser.restore()
  }
})

test('low-to-high quality changes resync without an immediate burst or long stall', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()
  harness.latestProps.current.qualityTier = 'low'
  const updates: number[] = []
  harness.renderer.update = (frame) => updates.push(frame.nowMs)

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    runNextFrame(browser, 0)
    runNextFrame(browser, 34)
    harness.latestProps.current.qualityTier = 'high'
    runNextFrame(browser, 40)
    runNextFrame(browser, 50.8)

    assert.deepEqual(updates, [0, 34, 50.8])
    cleanup()
  } finally {
    browser.restore()
  }
})

test('cleanup cancels a queued frame and ignores a late callback', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()
  let updates = 0
  harness.renderer.update = () => { updates += 1 }

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    const lateCallback = browser.calls.frames[0]
    cleanup()
    cleanup()
    lateCallback?.(0)

    assert.equal(updates, 0)
    assert.deepEqual(browser.calls.framesCancelled, [42])
  } finally {
    browser.restore()
  }
})

test('a long rAF gap does not immediately downgrade a stable high-quality orb', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    runCadence(browser, 0, 2500, 1000 / 17)
    assert.equal(harness.latestProps.current.qualityTier, 'high')

    browser.setNow(3734)
    runNextFrame(browser, 3734)

    assert.equal(harness.latestProps.current.qualityTier, 'high')
    cleanup()
  } finally {
    browser.restore()
  }
})

test('a rAF gap resets low-tier recovery until a fresh stable five seconds elapse', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()
  harness.latestProps.current.qualityTier = 'low'

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    runCadence(browser, 0, 2500, 60)
    assert.equal(harness.latestProps.current.qualityTier, 'low')

    browser.setNow(3700)
    runNextFrame(browser, 3700)
    runCadence(browser, 3700 + (1000 / 60), 8600, 60)
    assert.equal(harness.latestProps.current.qualityTier, 'low')

    runCadence(browser, 8600 + (1000 / 60), 8800, 60)
    assert.equal(harness.latestProps.current.qualityTier, 'high')
    cleanup()
  } finally {
    browser.restore()
  }
})

test('a forced-low policy applied after a rAF gap remains low', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    runCadence(browser, 0, 300, 60)

    browser.setNow(1500)
    browser.setReducedMotion(true)
    runNextFrame(browser, 1500)

    assert.equal(harness.latestProps.current.qualityTier, 'low')
    cleanup()
  } finally {
    browser.restore()
  }
})

test('normal no-gap frame cadence still degrades and recovers quality', () => {
  const browser = installBrowserFakes(false, { captureFrames: true })
  const harness = createHarness()

  try {
    const cleanup = startOrbLifecycle(
      harness.renderer,
      harness.canvas,
      harness.latestProps,
      harness.onFallback,
    )
    runCadence(browser, 0, 2200, 44)
    assert.equal(harness.latestProps.current.qualityTier, 'low')

    runCadence(browser, 2200 + (1000 / 60), 9500, 60)
    assert.equal(harness.latestProps.current.qualityTier, 'high')
    cleanup()
  } finally {
    browser.restore()
  }
})
