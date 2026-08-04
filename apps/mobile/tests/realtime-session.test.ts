import assert from 'node:assert/strict'
import test from 'node:test'

import { RealtimeSession } from '../src/realtime/RealtimeSession.ts'
import {
  REALTIME_PROTOCOL_VERSION,
  createTurnId,
  createRequestedFrameEvents,
  createSessionStart,
} from '../src/realtime/protocol.ts'

test('session start declares protocol version and native build', () => {
  const event = createSessionStart('video')

  assert.equal(event.protocol_version, 4)
  assert.equal(event.client_build.length > 0, true)
  assert.equal(REALTIME_PROTOCOL_VERSION, 4)
  assert.equal('activation_mode' in event, false)
})

test('turn ids are non-empty and unique', () => {
  const first = createTurnId()
  const second = createTurnId()

  assert.equal(first.length > 0, true)
  assert.equal(second.length > 0, true)
  assert.notEqual(first, second)
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

function readySessionHarness() {
  const sent: Array<Record<string, unknown>> = []
  const errors: string[] = []
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode: 'audio',
    onState: () => {},
    onError: (message) => errors.push(message),
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
    ready: boolean
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
    handleText(text: string): void
  }
  internals.ready = true
  internals.transport = {
    send: async (message) => sent.push(JSON.parse(message)),
    close: async () => {},
  }
  return {
    session,
    receive: (event: Record<string, unknown>) =>
      internals.handleText(JSON.stringify(event)),
    sent,
    errors,
  }
}

test('continue decision waits exactly 1.5 seconds before committing', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted()
  session.speechPaused()
  receive({
    type: 'input.turn.decision',
    turn_id: sent.at(-1)?.turn_id,
    decision: 'continue',
  })

  t.mock.timers.tick(1499)
  assert.equal(sent.some((event) => event.type === 'input.commit'), false)
  t.mock.timers.tick(1)
  await new Promise((resolve) => setImmediate(resolve))

  assert.equal(sent.at(-1)?.type, 'input.commit')
})

test('speech resumption cancels a pending endpoint timer', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()
  receive({
    type: 'input.turn.decision',
    turn_id: turnId,
    decision: 'uncertain',
  })

  await session.speechStarted()

  assert.deepEqual(sent.at(-1), {
    type: 'input.speech_resumed',
    turn_id: turnId,
  })
  t.mock.timers.tick(1501)
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(sent.some((event) => event.type === 'input.commit'), false)
})

test('failed pause send closes the session without an unhandled rejection', async () => {
  const { session, sent, errors } = readySessionHarness()
  let closeCalls = 0
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
  }
  internals.transport = {
    send: async (message) => {
      const event = JSON.parse(message) as Record<string, unknown>
      sent.push(event)
      if (event.type === 'input.turn.pause') throw new Error('pause failed')
    },
    close: async () => {
      closeCalls += 1
    },
  }

  await session.speechStarted()
  session.speechPaused()
  await new Promise((resolve) => setImmediate(resolve))
  await new Promise((resolve) => setImmediate(resolve))
  await session.speechStarted()

  assert.deepEqual(errors, ['pause failed'])
  assert.equal(closeCalls, 1)
  assert.deepEqual(sent.map((event) => event.type), [
    'input.speech_started',
    'input.turn.pause',
  ])
})

test('failed endpoint commit closes the session without an unhandled rejection', async () => {
  const { session, receive, sent, errors } = readySessionHarness()
  let closeCalls = 0
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
  }
  internals.transport = {
    send: async (message) => {
      const event = JSON.parse(message) as Record<string, unknown>
      sent.push(event)
      if (event.type === 'input.commit') throw new Error('commit failed')
    },
    close: async () => {
      closeCalls += 1
    },
  }

  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()
  receive({ type: 'input.turn.decision', turn_id: turnId, decision: 'complete' })
  await new Promise((resolve) => setImmediate(resolve))
  await new Promise((resolve) => setImmediate(resolve))
  await session.speechStarted()

  assert.deepEqual(errors, ['commit failed'])
  assert.equal(closeCalls, 1)
  assert.deepEqual(sent.map((event) => event.type), [
    'input.speech_started',
    'input.turn.pause',
    'input.commit',
  ])
})

test('failed pause prevents an immediately queued automatic commit from sending', async () => {
  const { session, receive, sent, errors } = readySessionHarness()
  let closeCalls = 0
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
  }
  internals.transport = {
    send: async (message) => {
      const event = JSON.parse(message) as Record<string, unknown>
      sent.push(event)
      if (event.type === 'input.turn.pause') throw new Error('pause failed')
    },
    close: async () => {
      closeCalls += 1
    },
  }

  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()
  receive({ type: 'input.turn.decision', turn_id: turnId, decision: 'complete' })
  await new Promise((resolve) => setImmediate(resolve))
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(errors, ['pause failed'])
  assert.equal(closeCalls, 1)
  assert.deepEqual(sent.map((event) => event.type), [
    'input.speech_started',
    'input.turn.pause',
  ])
})

test('stale decisions and handled commands cannot affect the pending turn', async () => {
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()

  receive({ type: 'input.command.handled', turn_id: 'stale-turn', command: 'stop' })
  receive({ type: 'input.turn.decision', turn_id: 'stale-turn', decision: 'complete' })
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(sent.at(-1)?.type, 'input.turn.pause')

  receive({ type: 'input.turn.decision', turn_id: turnId, decision: 'complete' })
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(sent.at(-1), { type: 'input.commit', turn_id: turnId })
})

test('pause and commit wait behind already queued audio appends', async () => {
  const sent: Array<Record<string, unknown>> = []
  let releaseFirstAppend: (() => void) | null = null
  const firstAppend = new Promise<void>((resolve) => {
    releaseFirstAppend = resolve
  })
  const { session } = readySessionHarness()
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
  }
  internals.transport = {
    send: async (message) => {
      const event = JSON.parse(message) as Record<string, unknown>
      sent.push(event)
      if (event.type === 'input.audio.append' && sent.length === 2) {
        await firstAppend
      }
    },
    close: async () => {},
  }

  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  void session.sendInput(new Float32Array([0.1]))
  void session.sendInput(new Float32Array([0.2]))
  session.speechPaused()
  ;(session as unknown as { handleText(text: string): void }).handleText(
    JSON.stringify({
      type: 'input.turn.decision',
      turn_id: turnId,
      decision: 'complete',
    }),
  )

  releaseFirstAppend?.()
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(
    sent.slice(-4).map((event) => event.type),
    [
      'input.audio.append',
      'input.audio.append',
      'input.turn.pause',
      'input.commit',
    ],
  )
})

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
    {
      type: 'input.speech_started',
      turn_id: sent[1]?.turn_id,
    },
  ])
  assert.equal(typeof sent[1]?.turn_id, 'string')
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
    {
      type: 'input.speech_started',
      turn_id: sent[2]?.turn_id,
    },
  ])
  assert.equal(typeof sent[2]?.turn_id, 'string')
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
