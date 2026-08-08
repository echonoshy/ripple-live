import assert from 'node:assert/strict'
import test from 'node:test'

import { RealtimeSession } from '../src/realtime/RealtimeSession.ts'
import type { ToolCompletion } from '../src/realtime/toolResults.ts'
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

function failureHarness({
  onToolResult,
}: {
  onToolResult?: (result: ToolCompletion) => void
} = {}) {
  const states: string[] = []
  const tools: string[] = []
  const assistantTexts: string[] = []
  const errors: string[] = []
  const responseFailures: string[] = []
  const results: ToolCompletion[] = []
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
    onToolResult: (result) => {
      results.push(result)
      onToolResult?.(result)
    },
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
    results,
    get audioClears() {
      return audioClears
    },
  }
}

test('emits one correlated completed tool result without altering opaque fields', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: ' call-1 ',
    name: ' remember ',
    result: { ok: true },
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: ' call-1 ',
    name: ' remember ',
    result: { ok: true },
  })

  assert.deepEqual(harness.results, [
    { callId: ' call-1 ', name: ' remember ', result: { ok: true } },
  ])
})

test('ignores stale, uncorrelated, and blank tool results', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-current' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'stale',
    call_id: 'call-1',
    name: 'remember',
    result: { ok: true },
  })
  harness.receive({
    type: 'response.tool.completed',
    call_id: 'call-2',
    name: 'remember',
    result: { ok: true },
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-current',
    call_id: '  ',
    name: 'remember',
    result: { ok: true },
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-current',
    call_id: 'call-3',
    name: '\t',
    result: { ok: true },
  })

  assert.deepEqual(harness.results, [])
  assert.deepEqual(harness.tools, [''])
})

test('does not re-emit tool results after response completion', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: 'call-1',
    name: 'remember',
    result: { ok: true },
  })
  harness.receive({ type: 'response.done', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: 'call-2',
    name: 'remember',
    result: { ok: true },
  })

  assert.deepEqual(harness.results, [
    { callId: 'call-1', name: 'remember', result: { ok: true } },
  ])
  assert.equal(harness.states.at(-1), 'listening')
})

test('contains a tool result callback exception and continues session transport', () => {
  const harness = failureHarness({
    onToolResult: () => {
      throw new Error('callback failed')
    },
  })
  harness.receive({ type: 'response.created', response_id: 'response-1' })

  assert.doesNotThrow(() => {
    harness.receive({
      type: 'response.tool.completed',
      response_id: 'response-1',
      call_id: 'call-1',
      name: 'remember',
      result: { ok: true },
    })
  })
  harness.receive({
    type: 'response.text.delta',
    response_id: 'response-1',
    delta: 'still connected',
  })

  assert.deepEqual(harness.results, [
    { callId: 'call-1', name: 'remember', result: { ok: true } },
  ])
  assert.deepEqual(harness.assistantTexts, ['still connected'])
})

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
    onToolResult: () => {},
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
  assert.deepEqual(sent.at(-1), {
    type: 'input.commit',
    turn_id: sent[0]?.turn_id,
    endpoint_fallback: true,
  })
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
  assert.deepEqual(sent.at(-1), {
    type: 'input.commit',
    turn_id: turnId,
    endpoint_fallback: false,
  })
})

test('force listen clears a currently speaking client turn on the server', async () => {
  const { session, sent } = readySessionHarness()
  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id

  session.forceListen()
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent, [
    { type: 'input.speech_started', turn_id: turnId },
    { type: 'response.cancel', clear_input: true },
    { type: 'input.clear' },
  ])
})

test('force listen clears a pause-pending turn and rejects its delayed decision', async () => {
  const { session, receive, sent } = readySessionHarness()
  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()
  await new Promise((resolve) => setImmediate(resolve))

  session.forceListen()
  receive({ type: 'input.turn.decision', turn_id: turnId, decision: 'complete' })
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent, [
    { type: 'input.speech_started', turn_id: turnId },
    { type: 'input.turn.pause', turn_id: turnId },
    { type: 'response.cancel', clear_input: true },
    { type: 'input.clear' },
  ])
})

test('new speech waits until force-listen input clear reaches the server', async () => {
  const sent: Array<Record<string, unknown>> = []
  let releasePause: (() => void) | null = null
  const pauseHeld = new Promise<void>((resolve) => {
    releasePause = resolve
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
      if (event.type === 'input.turn.pause') await pauseHeld
    },
    close: async () => {},
  }

  await session.speechStarted()
  const firstTurnId = sent.at(-1)?.turn_id
  session.speechPaused()
  await new Promise((resolve) => setImmediate(resolve))
  session.forceListen()
  const nextSpeech = session.speechStarted()
  await new Promise((resolve) => setImmediate(resolve))

  releasePause?.()
  await nextSpeech
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent.map((event) => event.type), [
    'input.speech_started',
    'input.turn.pause',
    'response.cancel',
    'input.clear',
    'input.speech_started',
  ])
  assert.equal(sent[0]?.turn_id, firstTurnId)
  assert.notEqual(sent[4]?.turn_id, firstTurnId)
  assert.equal(sent[2]?.clear_input, true)
})

test('force listen clears a commit that is already queued for transport', async () => {
  const sent: Array<Record<string, unknown>> = []
  let releasePause: (() => void) | null = null
  const pauseHeld = new Promise<void>((resolve) => {
    releasePause = resolve
  })
  const { session, receive } = readySessionHarness()
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
      if (event.type === 'input.turn.pause') await pauseHeld
    },
    close: async () => {},
  }

  await session.speechStarted()
  const turnId = sent.at(-1)?.turn_id
  session.speechPaused()
  await new Promise((resolve) => setImmediate(resolve))
  receive({ type: 'input.turn.decision', turn_id: turnId, decision: 'complete' })
  session.forceListen()

  releasePause?.()
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent.map((event) => event.type), [
    'input.speech_started',
    'input.turn.pause',
    'response.cancel',
    'input.commit',
    'input.clear',
  ])
  assert.equal(sent[2]?.clear_input, true)
})

test('in-flight speech cancellation rechecks a force-listen clear before starting', async () => {
  const sent: Array<Record<string, unknown>> = []
  let releaseCancel: (() => void) | null = null
  const cancelHeld = new Promise<void>((resolve) => {
    releaseCancel = resolve
  })
  const { session, receive } = readySessionHarness()
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
  }
  let cancelCount = 0
  internals.transport = {
    send: async (message) => {
      const event = JSON.parse(message) as Record<string, unknown>
      sent.push(event)
      if (event.type === 'response.cancel' && ++cancelCount === 1) {
        await cancelHeld
      }
    },
    close: async () => {},
  }
  receive({ type: 'response.created', response_id: 'response-in-flight' })

  const starting = session.speechStarted()
  await new Promise((resolve) => setImmediate(resolve))
  session.forceListen()
  releaseCancel?.()
  await starting
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(sent.map((event) => event.type), [
    'response.cancel',
    'response.cancel',
    'input.clear',
    'input.speech_started',
  ])
  assert.deepEqual(sent.slice(0, 3), [
    { type: 'response.cancel' },
    { type: 'response.cancel', clear_input: true },
    { type: 'input.clear' },
  ])
  assert.equal(typeof sent[3]?.turn_id, 'string')
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
    onToolResult: () => {},
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
    onToolResult: () => {},
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
    onToolResult: () => {},
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
