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
  const events = new EventTarget()
  return {
    srcObject: null as MediaStream | null,
    style: { transform: '' },
    play: async () => {},
    pause: () => {},
    addEventListener: events.addEventListener.bind(events),
    removeEventListener: events.removeEventListener.bind(events),
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
    initialVideo: false,
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

test('legacy initialVideo starts microphone and playback before requesting camera', async (t) => {
  const requests: MediaStreamConstraints[] = []
  const microphoneTrack = { kind: 'audio', stops: 0, stop() { this.stops += 1 } }
  const cameraTrack = {
    kind: 'video',
    readyState: 'live',
    stops: 0,
    stop() { this.stops += 1 },
    addEventListener() {},
    removeEventListener() {},
  }
  const microphone = {
    getTracks: () => [microphoneTrack],
    getVideoTracks: () => [],
  } as unknown as MediaStream
  const camera = {
    getTracks: () => [cameraTrack],
    getVideoTracks: () => [cameraTrack],
  } as unknown as MediaStream
  installNavigator(t, async (constraints) => {
    requests.push(constraints)
    return constraints.video === false ? microphone : camera
  })
  installAudio(t)
  const originalVadNew = MicVAD.new
  MicVAD.new = async () => ({ destroy: async () => {} }) as MicVAD
  t.after(() => { MicVAD.new = originalVadNew })
  const video = fakeVideo()
  Object.defineProperty(video, 'readyState', { configurable: true, value: 2 })
  const media = new LiveMedia({
    video,
    canvas: {} as HTMLCanvasElement,
    initialVideo: true,
    facingMode: 'environment',
    onPlaybackStarted: () => {},
    onPlaybackEnded: () => {},
    onOutputLevel: () => {},
  })

  await media.start(() => {}, () => {}, () => {}, () => {})
  assert.equal(requests.length, 2)
  assert.equal(requests[0].video, false)
  assert.deepEqual(requests[1].video, {
    facingMode: { ideal: 'environment' },
    width: { ideal: 1280 },
    height: { ideal: 720 },
  })
  assert.equal(media.cameraEnabled, true)

  media.disableCamera()
  assert.equal(cameraTrack.stops, 1)
  assert.equal(microphoneTrack.stops, 0)
  media.stop()
  assert.equal(microphoneTrack.stops, 1)
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
