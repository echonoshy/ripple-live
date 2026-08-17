import assert from 'node:assert/strict'
import test from 'node:test'

import {
  CameraController,
  waitForFirstFrame,
} from '../src/media/CameraController.ts'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

class FakeTrack extends EventTarget {
  stops = 0
  constructor(readonly kind: 'audio' | 'video') { super() }
  stop() { this.stops += 1 }
}

function fakeStream(name: string, withAudio = false) {
  const videoTrack = new FakeTrack('video')
  const audioTrack = withAudio ? new FakeTrack('audio') : null
  const tracks = audioTrack ? [videoTrack, audioTrack] : [videoTrack]
  return {
    name,
    stream: {
      getTracks: () => tracks,
      getVideoTracks: () => [videoTrack],
    } as unknown as MediaStream,
    videoTrack,
    audioTrack,
  }
}

class FakeVideo extends EventTarget {
  srcObject: MediaStream | null = null
  style = { transform: '' }
  readyState = 0
  videoWidth = 0
  videoHeight = 0
  playCalls = 0
  pauseCalls = 0
  async play() { this.playCalls += 1 }
  pause() { this.pauseCalls += 1 }
}

function createFakeTimers() {
  const entries: Array<{ active: boolean; callback: () => void; timeoutMs: number }> = []
  return {
    entries,
    setTimeout(callback: () => void, timeoutMs: number) {
      entries.push({ active: true, callback, timeoutMs })
      return entries.length - 1
    },
    clearTimeout(handle: unknown) {
      const entry = entries[handle as number]
      if (entry) entry.active = false
    },
    run(handle: number) {
      const entry = entries[handle]
      if (!entry?.active) return
      entry.active = false
      entry.callback()
    },
  }
}

function createHarness(timers = createFakeTimers()) {
  const requests: Array<{
    constraints: MediaStreamConstraints
    result: ReturnType<typeof deferred<MediaStream>>
  }> = []
  const frameWaits: Array<ReturnType<typeof deferred<void>>> = []
  const interruptions: string[] = []
  const video = new FakeVideo()
  const controller = new CameraController(video as unknown as HTMLVideoElement, {
    getUserMedia: (constraints) => {
      const result = deferred<MediaStream>()
      requests.push({ constraints, result })
      return result.promise
    },
    waitForFirstFrame: async () => {
      const result = deferred<void>()
      frameWaits.push(result)
      return result.promise
    },
    onInterrupted: () => interruptions.push('interrupted'),
    interruptionTimers: timers,
  })
  return { controller, frameWaits, interruptions, requests, timers, video }
}

test('camera is not requested before explicit enable', () => {
  const { controller, requests } = createHarness()
  assert.equal(controller.enabled, false)
  assert.equal(requests.length, 0)
})

test('enable requests environment video and resolves after the first frame', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const camera = fakeStream('camera')
  const enabling = controller.enable('environment')
  assert.deepEqual(requests[0].constraints, {
    audio: false,
    video: {
      facingMode: { ideal: 'environment' },
      width: { ideal: 1280 },
      height: { ideal: 720 },
    },
  })
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  assert.equal(controller.enabled, false)
  assert.equal(video.srcObject, camera.stream)
  frameWaits[0].resolve()
  assert.equal(await enabling, 'enabled')
  assert.equal(controller.enabled, true)
})

test('disable is idempotent and stops only video tracks', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const camera = fakeStream('camera', true)
  const enabling = controller.enable('user')
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await enabling

  controller.disable()
  controller.disable()
  assert.equal(camera.videoTrack.stops, 1)
  assert.equal(camera.audioTrack?.stops, 0)
  assert.equal(video.srcObject, null)
  assert.equal(controller.enabled, false)
})

test('disable disposes a getUserMedia result that arrives late', async () => {
  const { controller, requests, video } = createHarness()
  const late = fakeStream('late')
  const enabling = controller.enable('environment')
  controller.disable()
  requests[0].result.resolve(late.stream)

  assert.equal(await enabling, 'stale')
  assert.equal(late.videoTrack.stops, 1)
  assert.equal(video.srcObject, null)
  assert.equal(controller.enabled, false)
})

test('disable during first-frame wait cannot be undone by late readiness', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const late = fakeStream('late')
  const enabling = controller.enable('environment')
  requests[0].result.resolve(late.stream)
  await Promise.resolve()
  controller.disable()
  assert.equal(late.videoTrack.stops, 1)
  frameWaits[0].resolve()

  assert.equal(await enabling, 'stale')
  assert.equal(late.videoTrack.stops, 1)
  assert.equal(video.srcObject, null)
  assert.equal(controller.enabled, false)
})

test('same-mode enable is idempotent while pending and after success', async () => {
  const { controller, frameWaits, requests } = createHarness()
  const camera = fakeStream('camera')
  const first = controller.enable('environment')
  const duplicate = controller.enable('environment')
  assert.equal(requests.length, 1)
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  assert.equal(await first, 'enabled')
  assert.equal(await duplicate, 'enabled')
  assert.equal(await controller.enable('environment'), 'enabled')
  assert.equal(requests.length, 1)
})

test('latest overlapping enable wins and disposes the stale stream', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const stale = fakeStream('stale')
  const latest = fakeStream('latest')
  const first = controller.enable('user')
  const second = controller.enable('environment')
  requests[1].result.resolve(latest.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  assert.equal(await second, 'enabled')
  requests[0].result.resolve(stale.stream)

  assert.equal(await first, 'stale')
  assert.equal(stale.videoTrack.stops, 1)
  assert.equal(latest.videoTrack.stops, 0)
  assert.equal(video.srcObject, latest.stream)
})

test('requesting the active mode cancels a pending flip and restores its preview', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const current = fakeStream('current')
  const stale = fakeStream('stale')
  const opening = controller.enable('environment')
  requests[0].result.resolve(current.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening

  const first = controller.enable('user')
  requests[1].result.resolve(stale.stream)
  await Promise.resolve()
  const second = controller.enable('environment')
  frameWaits[1].resolve()
  assert.equal(await first, 'stale')
  assert.equal(await second, 'enabled')
  assert.equal(video.srcObject, current.stream)
  assert.equal(requests.length, 2)
  assert.equal(current.videoTrack.stops, 0)
  assert.equal(stale.videoTrack.stops, 1)
})

test('failed replacement preserves the previous preview without leaking tracks', async () => {
  const { controller, frameWaits, requests, video } = createHarness()
  const current = fakeStream('current')
  const replacement = fakeStream('replacement')
  const opening = controller.enable('environment')
  requests[0].result.resolve(current.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening

  const flipping = controller.enable('user')
  requests[1].result.resolve(replacement.stream)
  await Promise.resolve()
  frameWaits[1].reject(new Error('first frame timeout'))
  await assert.rejects(flipping, /first frame timeout/)
  assert.equal(video.srcObject, current.stream)
  assert.equal(current.videoTrack.stops, 0)
  assert.equal(replacement.videoTrack.stops, 1)
  assert.equal(controller.enabled, true)
})

test('ended camera track disables only video and reports interruption', async () => {
  const { controller, frameWaits, interruptions, requests, video } = createHarness()
  const camera = fakeStream('camera', true)
  const opening = controller.enable('environment')
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  camera.videoTrack.dispatchEvent(new Event('ended'))

  assert.equal(controller.enabled, false)
  assert.equal(video.srcObject, null)
  assert.equal(camera.audioTrack?.stops, 0)
  assert.deepEqual(interruptions, ['interrupted'])
})

test('a transient mute that unmutes inside the grace period keeps the camera', async () => {
  const { controller, frameWaits, interruptions, requests, timers, video } = createHarness()
  const camera = fakeStream('camera', true)
  const opening = controller.enable('environment')
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  camera.videoTrack.dispatchEvent(new Event('mute'))
  assert.equal(timers.entries[0].timeoutMs, 1000)
  camera.videoTrack.dispatchEvent(new Event('unmute'))
  timers.run(0)

  assert.equal(controller.enabled, true)
  assert.equal(video.srcObject, camera.stream)
  assert.equal(camera.audioTrack?.stops, 0)
  assert.deepEqual(interruptions, [])
})

test('a sustained mute reports one interruption after the grace period', async () => {
  const { controller, frameWaits, interruptions, requests, timers, video } = createHarness()
  const camera = fakeStream('camera', true)
  const opening = controller.enable('environment')
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  camera.videoTrack.dispatchEvent(new Event('mute'))
  timers.run(0)
  camera.videoTrack.dispatchEvent(new Event('ended'))

  assert.equal(controller.enabled, false)
  assert.equal(video.srcObject, null)
  assert.equal(camera.audioTrack?.stops, 0)
  assert.deepEqual(interruptions, ['interrupted'])
})

test('disable during mute grace cancels interruption reporting', async () => {
  const { controller, frameWaits, interruptions, requests, timers } = createHarness()
  const camera = fakeStream('camera')
  const opening = controller.enable('environment')
  requests[0].result.resolve(camera.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  camera.videoTrack.dispatchEvent(new Event('mute'))
  controller.disable()
  timers.run(0)
  assert.deepEqual(interruptions, [])
})

test('starting a camera switch cancels the old track mute grace timer', async () => {
  const { controller, frameWaits, interruptions, requests, timers, video } = createHarness()
  const old = fakeStream('old')
  const current = fakeStream('current')
  const opening = controller.enable('environment')
  requests[0].result.resolve(old.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  old.videoTrack.dispatchEvent(new Event('mute'))

  const flipping = controller.enable('user')
  timers.run(0)
  requests[1].result.resolve(current.stream)
  await Promise.resolve()
  frameWaits[1].resolve()
  await flipping

  assert.equal(controller.enabled, true)
  assert.equal(video.srcObject, current.stream)
  assert.deepEqual(interruptions, [])
})

test('mute from a replaced old track cannot interrupt the current camera', async () => {
  const { controller, frameWaits, interruptions, requests, timers, video } = createHarness()
  const old = fakeStream('old')
  const current = fakeStream('current')
  const opening = controller.enable('environment')
  requests[0].result.resolve(old.stream)
  await Promise.resolve()
  frameWaits[0].resolve()
  await opening
  const flipping = controller.enable('user')
  requests[1].result.resolve(current.stream)
  await Promise.resolve()
  frameWaits[1].resolve()
  await flipping
  old.videoTrack.dispatchEvent(new Event('mute'))
  for (let index = 0; index < timers.entries.length; index += 1) timers.run(index)

  assert.equal(controller.enabled, true)
  assert.equal(video.srcObject, current.stream)
  assert.deepEqual(interruptions, [])
})

for (const lateFrame of ['resolve', 'reject', 'timeout'] as const) {
  test(`current track ending during a pending flip survives late ${lateFrame} without ghost preview`, async () => {
    const { controller, frameWaits, interruptions, requests, video } = createHarness()
    const current = fakeStream('current')
    const pending = fakeStream('pending')
    const opening = controller.enable('environment')
    requests[0].result.resolve(current.stream)
    await Promise.resolve()
    frameWaits[0].resolve()
    await opening

    const flipping = controller.enable('user')
    const outcome = flipping.then(
      (result) => result,
      (error: unknown) => `rejected:${String(error)}`,
    )
    requests[1].result.resolve(pending.stream)
    await Promise.resolve()
    assert.equal(video.srcObject, pending.stream)
    current.videoTrack.dispatchEvent(new Event('ended'))
    if (lateFrame === 'resolve') frameWaits[1].resolve()
    else frameWaits[1].reject(new Error(lateFrame))

    assert.equal(await outcome, 'stale')
    assert.equal(controller.enabled, false)
    assert.equal(video.srcObject, null)
    assert.equal(current.videoTrack.stops, 1)
    assert.equal(pending.videoTrack.stops, 1)
    assert.deepEqual(interruptions, ['interrupted'])
  })
}

test('first-frame waiter resolves immediately for an already playable video', async () => {
  const video = new FakeVideo()
  video.readyState = 2
  video.videoWidth = 1280
  const timers: number[] = []
  await waitForFirstFrame(video as unknown as HTMLVideoElement, 3000, {
    setTimeout: () => { timers.push(1); return 1 },
    clearTimeout: () => {},
  })
  assert.equal(video.playCalls, 1)
  assert.equal(timers.length, 0)
})

test('first-frame waiter cleans listeners and timeout on loadeddata and error', async () => {
  for (const result of ['loadeddata', 'error'] as const) {
    const video = new FakeVideo()
    let timeoutCallback = () => {}
    const cleared: unknown[] = []
    const waiting = waitForFirstFrame(video as unknown as HTMLVideoElement, 3000, {
      setTimeout: (callback) => { timeoutCallback = callback; return 17 },
      clearTimeout: (handle) => { cleared.push(handle) },
    })
    video.dispatchEvent(new Event(result))
    if (result === 'loadeddata') await waiting
    else await assert.rejects(waiting, /摄像头画面加载失败/)
    assert.deepEqual(cleared, [17])
    timeoutCallback()
  }
})

test('first-frame waiter clears a timer when play synchronously emits loadeddata', async () => {
  const video = new FakeVideo()
  const cleared: unknown[] = []
  video.play = async () => {
    video.dispatchEvent(new Event('loadeddata'))
  }
  await waitForFirstFrame(video as unknown as HTMLVideoElement, 3000, {
    setTimeout: () => 29,
    clearTimeout: (handle) => { cleared.push(handle) },
  })
  assert.deepEqual(cleared, [29])
})

test('first-frame waiter rejects deterministically on timeout', async () => {
  const video = new FakeVideo()
  let timeoutCallback = () => {}
  const waiting = waitForFirstFrame(video as unknown as HTMLVideoElement, 3000, {
    setTimeout: (callback) => { timeoutCallback = callback; return 23 },
    clearTimeout: () => {},
  })
  timeoutCallback()
  await assert.rejects(waiting, /摄像头画面加载超时/)
})

test('first-frame waiter abort cleans its listeners and timer immediately', async () => {
  const video = new FakeVideo()
  const abort = new AbortController()
  const cleared: unknown[] = []
  const waiting = waitForFirstFrame(video as unknown as HTMLVideoElement, 3000, {
    setTimeout: () => 31,
    clearTimeout: (handle) => { cleared.push(handle) },
  }, abort.signal)
  abort.abort()
  await assert.rejects(waiting, { name: 'AbortError' })
  assert.deepEqual(cleared, [31])
})
