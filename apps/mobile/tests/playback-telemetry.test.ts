import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

import { LiveMedia } from '../src/media/LiveMedia.ts'

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

type PlaybackMessage = {
  type: string
  level?: number
}

function playbackHarness(sampleRate: number) {
  const messages: PlaybackMessage[] = []
  let Processor: new (options: unknown) => {
    port: { onmessage(event: { data: unknown }): void }
    process(inputs: unknown[], outputs: Float32Array[][]): boolean
  }

  class WorkletProcessor {
    port = {
      postMessage: (message: PlaybackMessage) => messages.push(message),
      onmessage: (_event: { data: unknown }) => {},
    }
  }

  vm.runInNewContext(
    readFileSync(path.join(appRoot, 'public/playback-processor.js'), 'utf8'),
    {
      AudioWorkletProcessor: WorkletProcessor,
      sampleRate,
      registerProcessor: (
        _name: string,
        processor: typeof Processor,
      ) => {
        Processor = processor
      },
      Math,
    },
  )

  const processor = new Processor({
    processorOptions: { initialBufferMs: 0, rebufferMs: 0 },
  })
  return {
    messages,
    send(data: unknown) {
      processor.port.onmessage({ data })
    },
    process(length: number) {
      const output = new Float32Array(length)
      processor.process([], [[output]])
      return output
    },
  }
}

function audioLevels(messages: PlaybackMessage[]) {
  return messages
    .filter((message) => message.type === 'audio-level')
    .map((message) => message.level)
}

test('worklet ends exactly when its final quantum is rendered', () => {
  const playback = playbackHarness(20)
  playback.send({ type: 'enqueue', samples: new Float32Array([0.25, 0.25]) })
  playback.send({ type: 'end' })

  playback.process(2)

  assert.equal(
    playback.messages.some((message) => message.type === 'playback-ended'),
    true,
  )
  assert.equal(audioLevels(playback.messages).at(-1), 0)
})

test('worklet ends when the final buffer only fills part of a quantum', () => {
  const playback = playbackHarness(20)
  playback.send({ type: 'enqueue', samples: new Float32Array([0.25]) })
  playback.send({ type: 'end' })

  playback.process(2)

  assert.equal(
    playback.messages.some((message) => message.type === 'playback-ended'),
    true,
  )
  assert.equal(audioLevels(playback.messages).at(-1), 0)
})

test('worklet ends after an underrun when end arrives with no queued audio', () => {
  const playback = playbackHarness(20)
  playback.send({ type: 'enqueue', samples: new Float32Array([0.25]) })
  playback.process(2)
  playback.messages.length = 0

  playback.send({ type: 'end' })
  playback.process(2)

  assert.equal(
    playback.messages.some((message) => message.type === 'playback-ended'),
    true,
  )
  assert.equal(audioLevels(playback.messages).at(-1), 0)
})

test('worklet clears its RMS window and emits zero when playback underruns', () => {
  const playback = playbackHarness(160)
  playback.send({ type: 'enqueue', samples: new Float32Array([0.5, 0.5]) })
  playback.process(4)
  playback.send({ type: 'enqueue', samples: new Float32Array(8).fill(0.1) })
  playback.process(4)
  playback.process(4)

  const levels = audioLevels(playback.messages)
  assert.equal(levels[0], 0)
  assert.ok(Math.abs((levels[1] ?? 0) - 0.6) < 0.000_001)
})

test('LiveMedia normalizes non-finite playback levels to zero', async (t) => {
  const originalAudioContext = Object.getOwnPropertyDescriptor(
    globalThis,
    'AudioContext',
  )
  const originalAudioWorkletNode = Object.getOwnPropertyDescriptor(
    globalThis,
    'AudioWorkletNode',
  )
  const nodes: Array<{
    port: {
      onmessage: ((event: MessageEvent<PlaybackMessage>) => void) | null
    }
  }> = []

  class FakeAudioContext {
    state = 'running'
    sampleRate = 24_000
    destination = {}
    audioWorklet = { addModule: async () => {} }
  }

  class FakeAudioWorkletNode {
    port = {
      postMessage: () => {},
      onmessage: null as ((event: MessageEvent<PlaybackMessage>) => void) | null,
    }

    constructor() {
      nodes.push(this)
    }

    connect() {}
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
    if (originalAudioContext) {
      Object.defineProperty(globalThis, 'AudioContext', originalAudioContext)
    } else {
      Reflect.deleteProperty(globalThis, 'AudioContext')
    }
    if (originalAudioWorkletNode) {
      Object.defineProperty(
        globalThis,
        'AudioWorkletNode',
        originalAudioWorkletNode,
      )
    } else {
      Reflect.deleteProperty(globalThis, 'AudioWorkletNode')
    }
  })

  const levels: number[] = []
  const media = new LiveMedia({
    video: {} as HTMLVideoElement,
    canvas: {} as HTMLCanvasElement,
    initialVideo: false,
    facingMode: 'user',
    onPlaybackStarted: () => {},
    onPlaybackEnded: () => {},
    onOutputLevel: (level) => levels.push(level),
  })
  await (media as unknown as { openPlayback(): Promise<void> }).openPlayback()

  for (const level of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    nodes[0].port.onmessage?.({
      data: { type: 'audio-level', level },
    } as MessageEvent<PlaybackMessage>)
  }

  assert.deepEqual(levels, [0, 0, 0])
})
