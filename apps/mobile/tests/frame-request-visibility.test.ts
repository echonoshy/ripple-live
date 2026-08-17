import assert from 'node:assert/strict'
import test from 'node:test'
import { createMinimumVisibleSignal } from '../src/live/frameRequestVisibility.ts'

function createTimers() {
  let now = 0
  let nextId = 0
  const tasks = new Map<number, { at: number; callback: () => void }>()
  return {
    advance(ms: number) {
      now += ms
      while (true) {
        const due = [...tasks.entries()]
          .filter(([, task]) => task.at <= now)
          .sort((left, right) => left[1].at - right[1].at)[0]
        if (!due) break
        tasks.delete(due[0])
        due[1].callback()
      }
    },
    clearTimeout(handle: unknown) {
      tasks.delete(handle as number)
    },
    now: () => now,
    setTimeout(callback: () => void, delayMs: number) {
      const id = ++nextId
      tasks.set(id, { at: now + delayMs, callback })
      return id
    },
  }
}

test('a queued frame request remains visible across a paint-sized 160ms window', () => {
  const timers = createTimers()
  const states: boolean[] = []
  const signal = createMinimumVisibleSignal({
    minimumMs: 160,
    onVisible: (visible) => states.push(visible),
    timers,
  })

  signal.update(true)
  signal.update(false)
  assert.deepEqual(states, [true])
  timers.advance(159)
  assert.deepEqual(states, [true])
  timers.advance(1)
  assert.deepEqual(states, [true, false])
})

test('a new frame request cancels and extends a pending hide', () => {
  const timers = createTimers()
  const states: boolean[] = []
  const signal = createMinimumVisibleSignal({
    minimumMs: 160,
    onVisible: (visible) => states.push(visible),
    timers,
  })

  signal.update(true)
  signal.update(false)
  timers.advance(100)
  signal.update(true)
  signal.update(false)
  timers.advance(60)
  assert.deepEqual(states, [true])
  timers.advance(99)
  assert.deepEqual(states, [true])
  timers.advance(1)
  assert.deepEqual(states, [true, false])
})

test('dispose cancels pending work and synchronously clears visible state', () => {
  const timers = createTimers()
  const states: boolean[] = []
  const signal = createMinimumVisibleSignal({
    minimumMs: 160,
    onVisible: (visible) => states.push(visible),
    timers,
  })

  signal.update(true)
  signal.update(false)
  signal.dispose()
  assert.deepEqual(states, [true, false])
  timers.advance(500)
  assert.deepEqual(states, [true, false])
  signal.update(true)
  assert.deepEqual(states, [true, false])
})
