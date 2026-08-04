import assert from 'node:assert/strict'
import test from 'node:test'

import { RealtimeSession } from '../src/realtime/RealtimeSession.ts'
import {
  REALTIME_PROTOCOL_VERSION,
  createRequestedFrameEvents,
  createSessionStart,
} from '../src/realtime/protocol.ts'

test('session start declares protocol version and native build', () => {
  const event = createSessionStart('video')

  assert.equal(event.protocol_version, 3)
  assert.equal(event.client_build.length > 0, true)
  assert.equal(REALTIME_PROTOCOL_VERSION, 3)
  assert.equal('activation_mode' in event, false)
})

function failureHarness() {
  const states: string[] = []
  const tools: string[] = []
  const assistantTexts: string[] = []
  const errors: string[] = []
  const responseFailures: string[] = []
  let audioClears = 0
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode: 'video',
    onState: (state) => states.push(state),
    onError: (message) => errors.push(message),
    onResponseFailed: (message) => responseFailures.push(message),
    onAssistantText: (text) => assistantTexts.push(text),
    onUserText: () => {},
    onTool: (label) => tools.push(label),
    onAudio: () => {},
    onAudioDone: () => {},
    onInterrupted: () => {
      audioClears += 1
    },
    onArtifact: () => {},
    onFrameRequested: () => null,
    onReady: async () => {},
    onConversation: () => {},
  })
  const receive = (event: Record<string, unknown>) =>
    (session as unknown as { handleText(text: string): void }).handleText(
      JSON.stringify(event),
    )
  return {
    receive,
    states,
    tools,
    assistantTexts,
    errors,
    responseFailures,
    get audioClears() {
      return audioClears
    },
  }
}

test('failed response clears partial output and returns continuous mode to listening', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-failed' })
  harness.receive({
    type: 'response.text.delta',
    response_id: 'response-failed',
    delta: '部分回答',
  })
  harness.receive({
    type: 'response.tool.started',
    response_id: 'response-failed',
    name: 'web_search',
  })

  harness.receive({
    type: 'response.failed',
    response_id: 'response-failed',
    code: 'agent_unavailable',
    message: 'Agent 服务暂时不可用',
  })

  assert.equal(harness.states.at(-1), 'listening')
  assert.equal(harness.tools.at(-1), '')
  assert.equal(harness.assistantTexts.at(-1), '')
  assert.equal(harness.audioClears, 1)
  assert.deepEqual(harness.errors, [])
  assert.deepEqual(harness.responseFailures, ['Agent 服务暂时不可用'])
})

test('playback start is reported once for the active response', async () => {
  const sent: Array<Record<string, unknown>> = []
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode: 'audio',
    onState: () => {},
    onError: () => {},
    onResponseFailed: () => {},
    onAssistantText: () => {},
    onUserText: () => {},
    onTool: () => {},
    onAudio: () => {},
    onAudioDone: () => {},
    onInterrupted: () => {},
    onArtifact: () => {},
    onFrameRequested: () => null,
    onReady: async () => {},
    onConversation: () => {},
  })
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
    handleText(text: string): void
  }
  internals.transport = {
    send: async (message) => sent.push(JSON.parse(message)),
    close: async () => {},
  }
  internals.handleText(
    JSON.stringify({ type: 'response.created', response_id: 'response-11' }),
  )

  session.outputPlaybackStarted(450)
  session.outputPlaybackStarted(450)
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent, [
    {
      type: 'output.playback.started',
      response_id: 'response-11',
      buffered_ms: 450,
    },
  ])
})

test('speech start immediately interrupts an active response before capturing input', async () => {
  const sent: Array<Record<string, unknown>> = []
  let audioClears = 0
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode: 'audio',
    onState: () => {},
    onError: () => {},
    onResponseFailed: () => {},
    onAssistantText: () => {},
    onUserText: () => {},
    onTool: () => {},
    onAudio: () => {},
    onAudioDone: () => {},
    onInterrupted: () => {
      audioClears += 1
    },
    onArtifact: () => {},
    onFrameRequested: () => null,
    onReady: async () => {},
    onConversation: () => {},
  })
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
    handleText(text: string): void
  }
  internals.transport = {
    send: async (message) => sent.push(JSON.parse(message)),
    close: async () => {},
  }
  internals.handleText(JSON.stringify({ type: 'session.ready' }))
  internals.handleText(
    JSON.stringify({ type: 'response.created', response_id: 'response-12' }),
  )

  await session.speechStarted()

  assert.equal(audioClears, 1)
  assert.deepEqual(sent, [
    { type: 'response.cancel' },
    { type: 'input.speech_started' },
  ])
})

test('speech start clears locally buffered playback after generation is already done', async () => {
  const sent: Array<Record<string, unknown>> = []
  let audioClears = 0
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode: 'audio',
    onState: () => {},
    onError: () => {},
    onResponseFailed: () => {},
    onAssistantText: () => {},
    onUserText: () => {},
    onTool: () => {},
    onAudio: () => {},
    onAudioDone: () => {},
    onInterrupted: () => {
      audioClears += 1
    },
    onArtifact: () => {},
    onFrameRequested: () => null,
    onReady: async () => {},
    onConversation: () => {},
  })
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
    handleText(text: string): void
  }
  internals.transport = {
    send: async (message) => sent.push(JSON.parse(message)),
    close: async () => {},
  }
  internals.handleText(JSON.stringify({ type: 'session.ready' }))
  internals.handleText(
    JSON.stringify({ type: 'response.created', response_id: 'response-buffered' }),
  )
  session.outputPlaybackStarted(450)
  internals.handleText(
    JSON.stringify({ type: 'response.done', response_id: 'response-buffered' }),
  )

  await session.speechStarted()

  assert.equal(audioClears, 1)
  assert.deepEqual(sent, [
    {
      type: 'output.playback.started',
      response_id: 'response-buffered',
      buffered_ms: 450,
    },
    { type: 'response.cancel' },
    { type: 'input.speech_started' },
  ])
})

test('requested frame and commit preserve one response id', () => {
  assert.deepEqual(
    createRequestedFrameEvents('response-7', 'jpeg-data', 1234),
    [
      {
        type: 'input.video.frame',
        response_id: 'response-7',
        image: 'jpeg-data',
        mime_type: 'image/jpeg',
        captured_at: 1234,
      },
      { type: 'input.video.commit', response_id: 'response-7' },
    ],
  )
})
