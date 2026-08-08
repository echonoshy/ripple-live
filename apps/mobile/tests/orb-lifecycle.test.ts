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
  | 'ResizeObserver'
  | 'requestAnimationFrame'
  | 'cancelAnimationFrame'

function installBrowserFakes(
  observeThrows: boolean,
  options: { reducedMotion?: boolean; captureFrames?: boolean } = {},
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
  const mediaQuery = {
    matches: options.reducedMotion ?? false,
    addEventListener: () => { calls.mediaAdded += 1 },
    removeEventListener: () => { calls.mediaRemoved += 1 },
  }

  class FakeResizeObserver {
    observe() {
      if (observeThrows) throw new Error('observe_failed')
    }
    disconnect() { calls.observerDisconnected += 1 }
  }

  setGlobal('window', { devicePixelRatio: 2, matchMedia: () => mediaQuery })
  setGlobal('navigator', {})
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

test('forced-low mode renders the orb at no more than 30fps', () => {
  const browser = installBrowserFakes(false, {
    reducedMotion: true,
    captureFrames: true,
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
    browser.calls.frames.shift()?.(0)
    browser.calls.frames.shift()?.(16)
    browser.calls.frames.shift()?.(34)

    assert.equal(harness.latestProps.current.qualityTier, 'low')
    assert.deepEqual(updates, [0, 34])
    cleanup()
  } finally {
    browser.restore()
  }
})
