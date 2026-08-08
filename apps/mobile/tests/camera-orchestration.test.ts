import assert from 'node:assert/strict'
import test from 'node:test'
import {
  createCameraOrchestrator,
  type CameraSnapshot,
} from '../src/live/cameraOrchestration.ts'

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, reject, resolve }
}

function createHarness() {
  const events: string[] = []
  const snapshots: CameraSnapshot[] = []
  const modeRequests: Array<{
    mode: 'audio' | 'video'
    request: ReturnType<typeof deferred<void>>
  }> = []
  const cameraRequests: Array<ReturnType<typeof deferred<'enabled' | 'stale'>>> = []
  const transitionRequests: Array<ReturnType<typeof deferred<void>>> = []

  const orchestrator = createCameraOrchestrator({
    enableCamera: () => {
      events.push('camera:enable')
      const request = deferred<'enabled' | 'stale'>()
      cameraRequests.push(request)
      return request.promise
    },
    disableCamera: () => events.push('camera:disable'),
    setMode: (mode) => {
      events.push(`mode:${mode}`)
      const request = deferred<void>()
      modeRequests.push({ mode, request })
      return request.promise
    },
    waitForTransition: () => {
      events.push('transition:wait')
      const request = deferred<void>()
      transitionRequests.push(request)
      return request.promise
    },
    onSnapshot: (snapshot) => snapshots.push(snapshot),
  })
  return {
    cameraRequests,
    events,
    modeRequests,
    orchestrator,
    snapshots,
    transitionRequests,
  }
}

test('camera opens only after first frame and matching video acknowledgement', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  assert.equal(harness.orchestrator.current().phase, 'opening')
  assert.deepEqual(harness.events, ['camera:enable'])

  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  assert.deepEqual(harness.events, ['camera:enable', 'mode:video'])
  assert.equal(harness.orchestrator.current().phase, 'opening')

  harness.modeRequests[0].request.resolve()
  assert.equal(await opening, 'on')
  assert.deepEqual(harness.orchestrator.current(), {
    phase: 'on',
    previewVisible: true,
    recovery: null,
    serverMode: 'video',
  })
})

test('failed video acknowledgement restores audio before disabling camera', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.reject(new Error('video timeout'))
  await Promise.resolve()
  await Promise.resolve()
  assert.deepEqual(harness.events, [
    'camera:enable',
    'mode:video',
    'mode:audio',
  ])
  harness.modeRequests[1].request.resolve()

  assert.equal(await opening, 'error')
  assert.deepEqual(harness.events, [
    'camera:enable',
    'mode:video',
    'mode:audio',
    'camera:disable',
  ])
  assert.equal(harness.orchestrator.current().previewVisible, false)
  assert.equal(harness.orchestrator.current().serverMode, 'audio')
  assert.equal(harness.orchestrator.current().recovery, 'open')
})

test('failed corrective audio acknowledgement keeps the preview and reports unknown mode', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.reject(new Error('video timeout'))
  await Promise.resolve()
  await Promise.resolve()
  harness.modeRequests[1].request.reject(new Error('audio timeout'))

  assert.equal(await opening, 'error')
  assert.equal(harness.events.includes('camera:disable'), false)
  assert.deepEqual(harness.orchestrator.current(), {
    phase: 'error',
    previewVisible: true,
    recovery: 'close',
    serverMode: 'unknown',
  })
})

test('camera closes only after audio acknowledgement and the 420ms transition', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.resolve()
  await opening

  const closing = harness.orchestrator.close()
  assert.equal(harness.orchestrator.current().phase, 'closing')
  harness.modeRequests[1].request.resolve()
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(harness.orchestrator.current().previewVisible, false)
  assert.equal(harness.events.at(-1), 'transition:wait')
  assert.equal(harness.events.includes('camera:disable'), false)

  harness.transitionRequests[0].resolve()
  assert.equal(await closing, 'off')
  assert.equal(harness.events.at(-1), 'camera:disable')
  assert.equal(harness.orchestrator.current().phase, 'off')
})

test('failed audio acknowledgement leaves camera visible for retry', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.resolve()
  await opening

  const closing = harness.orchestrator.close()
  harness.modeRequests[1].request.reject(new Error('audio timeout'))
  assert.equal(await closing, 'error')
  assert.equal(harness.events.includes('camera:disable'), false)
  assert.equal(harness.orchestrator.current().previewVisible, true)
  assert.equal(harness.orchestrator.current().recovery, 'close')

  const retry = harness.orchestrator.retry('user')
  assert.equal(harness.modeRequests[2].mode, 'audio')
  harness.modeRequests[2].request.resolve()
  await Promise.resolve()
  await Promise.resolve()
  harness.transitionRequests[0].resolve()
  assert.equal(await retry, 'off')
})

test('camera permission failure returns to the orb with an explicit open retry', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].reject(new Error('permission denied'))
  assert.equal(await opening, 'error')
  assert.deepEqual(harness.orchestrator.current(), {
    phase: 'error',
    previewVisible: false,
    recovery: 'open',
    serverMode: 'audio',
  })
  assert.deepEqual(harness.events, ['camera:enable', 'camera:disable'])

  const retry = harness.orchestrator.retry('user')
  harness.cameraRequests[1].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.resolve()
  assert.equal(await retry, 'on')
})

test('rapid taps coalesce and invalidation makes late completions inert', async () => {
  const harness = createHarness()
  const first = harness.orchestrator.open('environment')
  assert.equal(harness.orchestrator.open('user'), first)
  harness.orchestrator.invalidate()
  harness.cameraRequests[0].resolve('enabled')
  assert.equal(await first, 'stale')
  assert.deepEqual(harness.events, ['camera:enable'])
  assert.equal(harness.snapshots.at(-1)?.phase, 'off')
})

test('camera interruption corrects server audio without stopping the call', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  harness.modeRequests[0].request.resolve()
  await opening

  const interrupted = harness.orchestrator.interrupt()
  assert.deepEqual(harness.orchestrator.current(), {
    phase: 'error',
    previewVisible: false,
    recovery: 'audio',
    serverMode: 'unknown',
  })
  harness.modeRequests[1].request.resolve()
  assert.equal(await interrupted, 'off')
  assert.equal(harness.orchestrator.current().serverMode, 'audio')
  assert.equal(harness.events.includes('camera:disable'), false)
})

test('interruption during opening waits for a late video acknowledgement before confirming audio', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  assert.equal(harness.modeRequests[0].mode, 'video')

  const interrupted = harness.orchestrator.interrupt()
  assert.equal(harness.modeRequests.length, 1)
  assert.equal(harness.orchestrator.current().previewVisible, false)
  harness.modeRequests[0].request.resolve()
  assert.equal(await opening, 'stale')
  await Promise.resolve()
  assert.equal(harness.modeRequests[1].mode, 'audio')
  harness.modeRequests[1].request.resolve()

  assert.equal(await interrupted, 'off')
  assert.equal(harness.orchestrator.current().serverMode, 'audio')
  assert.equal(harness.events.includes('camera:disable'), false)
})

test('interruption during opening waits for video timeout before issuing corrective audio', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()

  const interrupted = harness.orchestrator.interrupt()
  assert.equal(harness.modeRequests.length, 1)
  harness.modeRequests[0].request.reject(new Error('video timeout'))
  assert.equal(await opening, 'stale')
  await Promise.resolve()
  assert.equal(harness.modeRequests[1].mode, 'audio')
  harness.modeRequests[1].request.resolve()

  assert.equal(await interrupted, 'off')
  assert.equal(harness.orchestrator.current().phase, 'off')
})

test('failed interruption correction stays unknown and retries audio without reopening camera', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  const interrupted = harness.orchestrator.interrupt()
  harness.modeRequests[0].request.resolve()
  assert.equal(await opening, 'stale')
  await Promise.resolve()
  harness.modeRequests[1].request.reject(new Error('audio timeout'))
  assert.equal(await interrupted, 'error')
  assert.deepEqual(harness.orchestrator.current(), {
    phase: 'error',
    previewVisible: false,
    recovery: 'audio',
    serverMode: 'unknown',
  })

  const retry = harness.orchestrator.retry('user')
  assert.equal(harness.modeRequests[2].mode, 'audio')
  harness.modeRequests[2].request.resolve()
  assert.equal(await retry, 'off')
  assert.equal(harness.cameraRequests.length, 1)
})

test('call invalidation cancels an interruption queued behind video mode work', async () => {
  const harness = createHarness()
  const opening = harness.orchestrator.open('environment')
  harness.cameraRequests[0].resolve('enabled')
  await Promise.resolve()
  const interrupted = harness.orchestrator.interrupt()
  harness.orchestrator.invalidate()
  harness.modeRequests[0].request.resolve()

  assert.equal(await opening, 'stale')
  assert.equal(await interrupted, 'stale')
  assert.equal(harness.modeRequests.length, 1)
  assert.equal(harness.orchestrator.current().phase, 'off')
})
