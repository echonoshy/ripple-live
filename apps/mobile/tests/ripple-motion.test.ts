import assert from 'node:assert/strict'
import test from 'node:test'
import * as rippleModule from '../src/live/ripple.ts'
import {
  RIPPLE_MOTION,
  advanceRipple,
  createRippleSignal,
  createRippleState,
  nextRippleSignalId,
} from '../src/live/ripple.ts'

test('uses the approved B/R2 dimensions and timing', () => {
  assert.deepEqual(RIPPLE_MOTION, {
    durationMs: 700,
    cooldownMs: 1200,
    startRadius: 1.03,
    endRadius: 1.28,
    maximumAlpha: 0.14,
  })
})

test('consumes dense events but starts only one ring inside the cooldown', () => {
  let state = createRippleState()
  let next = advanceRipple(state, { signal: createRippleSignal('speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false }, 1000)
  state = next.state
  assert.equal(next.frame.progress, 0)
  next = advanceRipple(state, { signal: createRippleSignal('tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 1300)
  assert.equal(next.frame.kind, 'speech')
  assert.ok(next.frame.progress > 0)
  next = advanceRipple(next.state, { signal: createRippleSignal('tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 2300)
  assert.equal(next.frame.kind, 'tool')
})

test('consumes a batched speech and interruption in order without stacking rings', () => {
  const enqueueSignal = Reflect.get(rippleModule, 'enqueueRippleSignal') as
    | undefined
    | ((signals: readonly ReturnType<typeof createRippleSignal>[], signal: ReturnType<typeof createRippleSignal>) => readonly ReturnType<typeof createRippleSignal>[])
  const advanceSignals = Reflect.get(rippleModule, 'advanceRippleSignals') as
    | undefined
    | ((state: ReturnType<typeof createRippleState>, input: {
        signals: readonly ReturnType<typeof createRippleSignal>[]
        visualState: 'listening'
        outputLevel: number
        reducedMotion: boolean
      }, nowMs: number) => ReturnType<typeof advanceRipple>)
  const consumeSignals = Reflect.get(rippleModule, 'consumeRippleSignalsThrough') as
    | undefined
    | ((signals: readonly ReturnType<typeof createRippleSignal>[], signalId: ReturnType<typeof createRippleSignal>['id']) => readonly ReturnType<typeof createRippleSignal>[])

  assert.equal(typeof enqueueSignal, 'function')
  assert.equal(typeof advanceSignals, 'function')
  assert.equal(typeof consumeSignals, 'function')
  if (!enqueueSignal || !advanceSignals || !consumeSignals) return

  const speech = createRippleSignal('speech')
  const interrupt = createRippleSignal('interrupt')
  let signals: readonly ReturnType<typeof createRippleSignal>[] = []
  signals = enqueueSignal(signals, speech)
  signals = enqueueSignal(signals, interrupt)

  assert.deepEqual(signals.map((signal) => signal.kind), ['speech', 'interrupt'])
  const next = advanceSignals(createRippleState(), {
    signals,
    visualState: 'listening',
    outputLevel: 0,
    reducedMotion: false,
  }, 1000)
  assert.equal(next.state.lastConsumedSignalId, interrupt.id)
  assert.equal(next.frame.kind, 'speech')
  assert.equal(next.frame.progress, 0)
  assert.equal(next.frame.haloPulse, 1)

  const interruptOnly = consumeSignals(signals, speech.id)
  assert.deepEqual(interruptOnly, [interrupt])
  assert.equal(
    consumeSignals(interruptOnly, speech.id),
    interruptOnly,
    'a stale acknowledgement should preserve queue identity for React bail-out',
  )
})

test('assistant output crosses the emphasis threshold once per speaking phrase', () => {
  let state = createRippleState()
  let next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.31, reducedMotion: false }, 2000)
  state = next.state
  assert.equal(next.frame.kind, 'assistant')
  next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.82, reducedMotion: false }, 3300)
  assert.equal(next.frame.kind, null)
  state = advanceRipple(next.state, { signal: null, visualState: 'listening', outputLevel: 0, reducedMotion: false }, 3400).state
  next = advanceRipple(state, { signal: null, visualState: 'speaking', outputLevel: 0.31, reducedMotion: false }, 4600)
  assert.equal(next.frame.kind, 'assistant')
})

test('reduced motion suppresses propagation and keeps a short halo pulse', () => {
  const next = advanceRipple(createRippleState(), { signal: createRippleSignal('speech'), visualState: 'listening', outputLevel: 0, reducedMotion: true }, 1000)
  assert.equal(next.frame.progress, null)
  assert.ok(next.frame.haloPulse > 0)
})

test('enabling reduced motion stops an active ring immediately', () => {
  let state = advanceRipple(
    createRippleState(),
    { signal: createRippleSignal('speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false },
    1000,
  ).state
  const next = advanceRipple(
    state,
    { signal: null, visualState: 'listening', outputLevel: 0, reducedMotion: true },
    1100,
  )

  assert.equal(next.frame.kind, null)
  assert.equal(next.frame.progress, null)
  assert.equal(next.state.activeKind, null)
})

test('tracks only the highest consumed incrementing signal ID', () => {
  let state = createRippleState()
  for (let id = 1; id <= 1000; id++) {
    state = advanceRipple(
      state,
      { signal: createRippleSignal('speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false },
      id * 1300,
    ).state
  }

  assert.ok(state.lastConsumedSignalId !== null)
  assert.equal('consumedSignalIds' in state, false)
})

test('creates globally unique, positive, monotonically increasing signal IDs', () => {
  const first = createRippleSignal('speech')
  const second = createRippleSignal('tool')
  assert.ok(first.id > 0)
  assert.equal(second.id, first.id + 1)
  assert.notEqual(first.id, second.id)
})

test('treats repeated and lower factory IDs as stale', () => {
  const first = createRippleSignal('speech')
  const second = createRippleSignal('tool')
  let state = advanceRipple(
    createRippleState(),
    { signal: second, visualState: 'tool', outputLevel: 0, reducedMotion: false },
    1000,
  ).state
  let next = advanceRipple(
    state,
    { signal: first, visualState: 'listening', outputLevel: 0, reducedMotion: false },
    2300,
  )
  assert.equal(next.frame.kind, null)
  assert.equal(next.state.lastConsumedSignalId, second.id)

  next = advanceRipple(
    next.state,
    { signal: second, visualState: 'tool', outputLevel: 0, reducedMotion: false },
    3600,
  )
  assert.equal(next.frame.kind, null)
})

test('does not expose a mutable reset for the global signal counter', () => {
  assert.equal('setRippleSignalIdForTesting' in rippleModule, false)
})

test('fails clearly when the pure signal ID transition reaches the safe integer limit', () => {
  assert.equal(nextRippleSignalId(Number.MAX_SAFE_INTEGER - 1), Number.MAX_SAFE_INTEGER)
  assert.throws(() => nextRippleSignalId(Number.MAX_SAFE_INTEGER), RangeError)
})
