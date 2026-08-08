import assert from 'node:assert/strict'
import test from 'node:test'
import {
  RIPPLE_MOTION,
  advanceRipple,
  createRippleState,
  type RippleSignal,
} from '../src/live/ripple.ts'

const signal = (id: number, kind: RippleSignal['kind']): RippleSignal => ({ id, kind })

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
  let next = advanceRipple(state, { signal: signal(1, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false }, 1000)
  state = next.state
  assert.equal(next.frame.progress, 0)
  next = advanceRipple(state, { signal: signal(2, 'tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 1300)
  assert.equal(next.frame.kind, 'speech')
  assert.ok(next.frame.progress > 0)
  next = advanceRipple(next.state, { signal: signal(3, 'tool'), visualState: 'tool', outputLevel: 0, reducedMotion: false }, 2300)
  assert.equal(next.frame.kind, 'tool')
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
  const next = advanceRipple(createRippleState(), { signal: signal(1, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: true }, 1000)
  assert.equal(next.frame.progress, null)
  assert.ok(next.frame.haloPulse > 0)
})

test('enabling reduced motion stops an active ring immediately', () => {
  let state = advanceRipple(
    createRippleState(),
    { signal: signal(1, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false },
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
      { signal: signal(id, 'speech'), visualState: 'listening', outputLevel: 0, reducedMotion: false },
      id * 1300,
    ).state
  }

  assert.equal(state.lastConsumedSignalId, 1000)
  assert.equal('consumedSignalIds' in state, false)
})
