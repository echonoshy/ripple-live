import assert from 'node:assert/strict'
import test, { type TestContext } from 'node:test'
import { MicVAD } from '@ricky0123/vad-web'

import { assetBlob } from '../src/api.ts'
import { LiveMedia } from '../src/media/LiveMedia.ts'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

function fakeStream(name: string) {
  const track = {
    name,
    stops: 0,
    stop() { this.stops += 1 },
  }
  return {
    stream: { getTracks: () => [track] } as unknown as MediaStream,
    track,
  }
}

function fakeVideo() {
  return {
    srcObject: null as MediaStream | null,
    style: { transform: '' },
    play: async () => {},
    pause: () => {},
    videoWidth: 1280,
    videoHeight: 720,
  } as unknown as HTMLVideoElement
}

function installNavigator(
  t: TestContext,
  getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>,
) {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { mediaDevices: { getUserMedia } },
  })
  t.after(() => {
    if (original) Object.defineProperty(globalThis, 'navigator', original)
    else Reflect.deleteProperty(globalThis, 'navigator')
  })
}

function installAudio(t: TestContext) {
  const originalContext = Object.getOwnPropertyDescriptor(globalThis, 'AudioContext')
  const originalNode = Object.getOwnPropertyDescriptor(globalThis, 'AudioWorkletNode')

  class FakeAudioContext {
    state = 'running'
    sampleRate = 24_000
    destination = {}
    audioWorklet = { addModule: async () => {} }
    createMediaStreamSource() {
      return { connect: () => {}, disconnect: () => {} }
    }
    createGain() {
      return {
        gain: { value: 1 },
        connect: () => {},
        disconnect: () => {},
      }
    }
    resume = async () => {}
    close = async () => {}
  }

  class FakeAudioWorkletNode {
    port = { postMessage: () => {}, onmessage: null }
    connect() {}
    disconnect() {}
  }

  Object.defineProperty(globalThis, 'AudioContext', {
    configurable: true,
    value: FakeAudioContext,
  })
  Object.defineProperty(globalThis, 'AudioWorkletNode', {
    configurable: true,
    value: FakeAudioWorkletNode,
  })
  t.after(() => {
    if (originalContext) Object.defineProperty(globalThis, 'AudioContext', originalContext)
    else Reflect.deleteProperty(globalThis, 'AudioContext')
    if (originalNode) Object.defineProperty(globalThis, 'AudioWorkletNode', originalNode)
    else Reflect.deleteProperty(globalThis, 'AudioWorkletNode')
  })
}

function createMedia(video = fakeVideo()) {
  return {
    video,
    media: new LiveMedia({
      video,
      canvas: {} as HTMLCanvasElement,
      withVideo: true,
      facingMode: 'user',
      onPlaybackStarted: () => {},
      onPlaybackEnded: () => {},
      onOutputLevel: () => {},
    }),
  }
}

test('stop during a pending start disposes the late microphone stream without rejecting', async (t) => {
  const request = deferred<MediaStream>()
  const microphone = fakeStream('microphone')
  installNavigator(t, () => request.promise)
  installAudio(t)

  const originalVadNew = MicVAD.new
  MicVAD.new = async () => ({ destroy: async () => {} }) as MicVAD
  t.after(() => { MicVAD.new = originalVadNew })

  const video = fakeVideo()
  const media = new LiveMedia({
    video,
    canvas: {} as HTMLCanvasElement,
    withVideo: false,
    facingMode: 'user',
    onPlaybackStarted: () => {},
    onPlaybackEnded: () => {},
    onOutputLevel: () => {},
  })
  const errors: unknown[] = []
  const starting = media.start(() => {}, () => {}, () => {}, () => {})
    .catch((error) => { errors.push(error) })

  await Promise.resolve()
  media.stop()
  request.resolve(microphone.stream)
  await starting

  assert.equal(microphone.track.stops, 1)
  assert.deepEqual(errors, [])
  assert.equal(video.srcObject, null)
})

test('stop during a pending camera flip disposes the late stream', async (t) => {
  const requests: Array<ReturnType<typeof deferred<MediaStream>>> = []
  installNavigator(t, () => {
    const request = deferred<MediaStream>()
    requests.push(request)
    return request.promise
  })
  const { media, video } = createMedia()
  const current = fakeStream('current')
  const late = fakeStream('late')

  const opening = media.setFacingMode('environment')
  requests[0].resolve(current.stream)
  assert.equal(await opening, 'switched')
  const flipping = media.setFacingMode('user')
  media.stop()
  requests[1].resolve(late.stream)

  assert.equal(await flipping, 'stale')
  assert.equal(current.track.stops, 1)
  assert.equal(late.track.stops, 1)
  assert.equal(video.srcObject, null)
})

test('latest overlapping camera flip wins and the stale stream is stopped', async (t) => {
  const requests: Array<ReturnType<typeof deferred<MediaStream>>> = []
  installNavigator(t, () => {
    const request = deferred<MediaStream>()
    requests.push(request)
    return request.promise
  })
  const { media, video } = createMedia()
  const current = fakeStream('current')
  const stale = fakeStream('stale')
  const latest = fakeStream('latest')

  const opening = media.setFacingMode('environment')
  requests[0].resolve(current.stream)
  assert.equal(await opening, 'switched')
  const first = media.setFacingMode('user')
  const second = media.setFacingMode('environment')
  requests[2].resolve(latest.stream)
  assert.equal(await second, 'switched')
  requests[1].resolve(stale.stream)

  assert.equal(await first, 'stale')
  assert.equal(video.srcObject, latest.stream)
  assert.equal(current.track.stops, 1)
  assert.equal(stale.track.stops, 1)
  assert.equal(latest.track.stops, 0)
})

test('a superseded flip play failure restores the old preview while latest acquisition waits', async (t) => {
  const requests: Array<ReturnType<typeof deferred<MediaStream>>> = []
  installNavigator(t, () => {
    const request = deferred<MediaStream>()
    requests.push(request)
    return request.promise
  })
  const { media, video } = createMedia()
  const current = fakeStream('current')
  const stale = fakeStream('stale')
  const latest = fakeStream('latest')

  const opening = media.setFacingMode('environment')
  requests[0].resolve(current.stream)
  assert.equal(await opening, 'switched')

  const stalePlay = deferred<void>()
  let playCalls = 0
  video.play = () => {
    playCalls += 1
    return playCalls === 1 ? stalePlay.promise : Promise.resolve()
  }
  const first = media.setFacingMode('user')
  requests[1].resolve(stale.stream)
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(video.srcObject, stale.stream)

  const second = media.setFacingMode('environment')
  stalePlay.reject(new Error('stale play failed'))
  assert.equal(await first, 'stale')
  assert.equal(video.srcObject, current.stream)
  assert.equal(current.track.stops, 0)
  assert.equal(stale.track.stops, 1)

  requests[2].resolve(latest.stream)
  assert.equal(await second, 'switched')
  assert.equal(video.srcObject, latest.stream)
  assert.equal(current.track.stops, 1)
})

test('failed camera flip preserves the current preview and resolves without rejection', async (t) => {
  const requests: Array<ReturnType<typeof deferred<MediaStream>>> = []
  installNavigator(t, () => {
    const request = deferred<MediaStream>()
    requests.push(request)
    return request.promise
  })
  const { media, video } = createMedia()
  const current = fakeStream('current')

  const opening = media.setFacingMode('environment')
  requests[0].resolve(current.stream)
  assert.equal(await opening, 'switched')
  const flipping = media.setFacingMode('user')
  requests[1].reject(new Error('camera unavailable'))

  assert.equal(await flipping, 'failed')
  assert.equal(video.srcObject, current.stream)
  assert.equal(current.track.stops, 0)
})

test('assetBlob forwards cancellation and rejects with AbortError', async (t) => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
  let receivedSignal: AbortSignal | undefined
  Object.defineProperty(globalThis, 'fetch', {
    configurable: true,
    value: (_url: string, init: RequestInit) => new Promise<Response>((_resolve, reject) => {
      receivedSignal = init.signal ?? undefined
      if (!receivedSignal) {
        reject(new Error('signal missing'))
        return
      }
      receivedSignal?.addEventListener('abort', () => {
        reject(new DOMException('Aborted', 'AbortError'))
      }, { once: true })
    }),
  })
  t.after(() => {
    if (original) Object.defineProperty(globalThis, 'fetch', original)
    else Reflect.deleteProperty(globalThis, 'fetch')
  })

  const controller = new AbortController()
  const loading = assetBlob('example.test', 'secret', '/asset', controller.signal)
  controller.abort()

  await assert.rejects(loading, { name: 'AbortError' })
  assert.equal(receivedSignal, controller.signal)
})
