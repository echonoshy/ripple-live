import assert from 'node:assert/strict'
import test from 'node:test'
import { createCameraActivationGuard } from '../src/live/cameraActivation.ts'

test('home video intent is consumed once only after an allowed ready activation', () => {
  const guard = createCameraActivationGuard(true)
  guard.transition('connecting')
  guard.transition('preparing')
  guard.transition('listening')
  const token = guard.begin()
  assert.notEqual(token, null)
  assert.deepEqual(guard.commit(token!), { cameraRequested: true })
  assert.equal(guard.commit(token!), null)
})

test('voice activation becomes ready without requesting camera', () => {
  const guard = createCameraActivationGuard(false)
  guard.transition('listening')
  const token = guard.begin()
  assert.deepEqual(guard.commit(token!), { cameraRequested: false })
})

for (const terminalState of ['ended', 'error'] as const) {
  test(`late media start after ${terminalState} cannot become ready or retain video intent`, () => {
    const guard = createCameraActivationGuard(true)
    guard.transition('listening')
    const token = guard.begin()
    guard.transition(terminalState)
    assert.equal(guard.commit(token!), null)

    guard.transition('listening')
    const retryToken = guard.begin()
    assert.deepEqual(guard.commit(retryToken!), { cameraRequested: false })
  })
}

test('a disallowed transition during pending media start invalidates its token', () => {
  const guard = createCameraActivationGuard(true)
  guard.transition('listening')
  const token = guard.begin()
  guard.transition('preparing')
  guard.transition('listening')
  assert.equal(guard.commit(token!), null)
})

test('invalidating an old session does not affect a replacement guard', () => {
  const oldGuard = createCameraActivationGuard(true)
  oldGuard.transition('listening')
  const oldToken = oldGuard.begin()
  oldGuard.invalidate()

  const replacement = createCameraActivationGuard(true)
  replacement.transition('listening')
  const replacementToken = replacement.begin()
  assert.equal(oldGuard.commit(oldToken!), null)
  assert.deepEqual(replacement.commit(replacementToken!), {
    cameraRequested: true,
  })
})
