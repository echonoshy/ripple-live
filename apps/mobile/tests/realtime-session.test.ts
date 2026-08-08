import assert from 'node:assert/strict'
import test from 'node:test'

import { RealtimeSession } from '../src/realtime/RealtimeSession.ts'
import type { ToolCompletion } from '../src/realtime/toolResults.ts'
import {
  REALTIME_PROTOCOL_VERSION,
  createModeSet,
  createTurnId,
  createRequestedFrameEvents,
  createSessionStart,
} from '../src/realtime/protocol.ts'

type FakeSocket = {
  url: string
  onopen: (() => void) | null
  onmessage: ((event: { data: string }) => void) | null
  onerror: (() => void) | null
  onclose: (() => void) | null
  sent: string[]
  sendResult: Promise<void> | null
  closes: number
  open(): void
  finishClose(): void
  message(event: Record<string, unknown>): void
}

function installDeferredWebSockets(t: Parameters<typeof test>[1] extends (context: infer T) => unknown ? T : never) {
  const sockets: FakeSocket[] = []
  const originalWebSocket = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket')
  const originalIsTauri = Object.getOwnPropertyDescriptor(globalThis, 'isTauri')

  class DeferredWebSocket implements FakeSocket {
    onopen: (() => void) | null = null
    onmessage: ((event: { data: string }) => void) | null = null
    onerror: (() => void) | null = null
    onclose: (() => void) | null = null
    sent: string[] = []
    sendResult: Promise<void> | null = null
    closes = 0

    constructor(readonly url: string) {
      sockets.push(this)
    }

    send(message: string) {
      this.sent.push(message)
      return this.sendResult ?? undefined
    }

    close() {
      this.closes += 1
    }

    open() {
      this.onopen?.()
    }

    finishClose() {
      this.onclose?.()
    }

    message(event: Record<string, unknown>) {
      this.onmessage?.({ data: JSON.stringify(event) })
    }
  }

  Object.defineProperty(globalThis, 'WebSocket', {
    configurable: true,
    value: DeferredWebSocket,
  })
  Object.defineProperty(globalThis, 'isTauri', {
    configurable: true,
    value: false,
  })
  t.after(() => {
    if (originalWebSocket) Object.defineProperty(globalThis, 'WebSocket', originalWebSocket)
    else Reflect.deleteProperty(globalThis, 'WebSocket')
    if (originalIsTauri) Object.defineProperty(globalThis, 'isTauri', originalIsTauri)
    else Reflect.deleteProperty(globalThis, 'isTauri')
  })

  return sockets
}

function connectingSession(
  states: string[],
  ready: { count: number },
  conversationId?: string,
  onConversation: (conversationId: string) => void = () => {},
) {
  return new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    conversationId,
    mode: 'audio',
    onState: (state) => states.push(state),
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
    onReady: async () => {
      ready.count += 1
    },
    onConversation,
  })
}

test('a new session omits conversation ownership from its realtime URL', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const session = connectingSession([], { count: 0 })

  const connecting = session.connect()
  assert.equal(sockets.length, 1)
  const url = new URL(sockets[0]?.url ?? '')
  assert.equal(url.searchParams.has('conversation_id'), false)

  sockets[0]?.open()
  await connecting
})

test('a resumed session encodes its opaque conversation ID exactly once', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const conversationId = 'conv/with ?&=% unicode 你好'
  const session = connectingSession([], { count: 0 }, conversationId)

  const connecting = session.connect()
  assert.equal(sockets.length, 1)
  const url = new URL(sockets[0]?.url ?? '')
  assert.deepEqual(url.searchParams.getAll('conversation_id'), [conversationId])
  assert.equal((sockets[0]?.url.match(/conversation_id=/g) ?? []).length, 1)

  sockets[0]?.open()
  await connecting
})

test('server conversation callbacks accept nonblank IDs and reject malformed replacements', () => {
  const confirmed: string[] = []
  const session = connectingSession(
    [],
    { count: 0 },
    'conv_existing',
    (conversationId) => confirmed.push(conversationId),
  )
  const receive = (event: Record<string, unknown>) =>
    (session as unknown as { handleText(text: string): void }).handleText(
      JSON.stringify(event),
    )

  receive({ type: 'session.created', conversation_id: 'conv_server' })
  receive({ type: 'session.created', conversation_id: '  ' })

  assert.deepEqual(confirmed, ['conv_server'])
})

test('close invalidates a pending browser connection and closes its late transport', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const states: string[] = []
  const ready = { count: 0 }
  const session = connectingSession(states, ready)

  const connecting = session.connect()
  assert.equal(sockets.length, 1)
  await session.close()
  sockets[0]?.open()
  await connecting
  sockets[0]?.message({ type: 'session.ready' })

  assert.equal(sockets[0]?.closes, 1)
  assert.deepEqual(sockets[0]?.sent, [])
  assert.deepEqual(states, ['connecting', 'ended'])
  assert.equal(ready.count, 0)
})

test('a late old connection cannot affect a newer session', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const oldStates: string[] = []
  const oldReady = { count: 0 }
  const oldSession = connectingSession(oldStates, oldReady)
  const oldConnecting = oldSession.connect()
  await oldSession.close()

  const newStates: string[] = []
  const newReady = { count: 0 }
  const newSession = connectingSession(newStates, newReady)
  const newConnecting = newSession.connect()
  sockets[1]?.open()
  await newConnecting
  sockets[1]?.message({ type: 'session.ready' })

  sockets[0]?.open()
  await oldConnecting
  sockets[0]?.message({ type: 'session.ready' })

  assert.equal(sockets[0]?.closes, 1)
  assert.equal(oldReady.count, 0)
  assert.deepEqual(oldStates, ['connecting', 'ended'])
  assert.equal(sockets[1]?.closes, 0)
  assert.equal(newReady.count, 1)
  assert.deepEqual(newStates, ['connecting', 'preparing', 'listening'])
})

test('a pending connect settles when its stale socket closes before opening', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const states: string[] = []
  const session = connectingSession(states, { count: 0 })

  const connecting = session.connect()
  await session.close()
  sockets[0]?.finishClose()

  const settled = await Promise.race([
    connecting.then(() => true),
    new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 0)),
  ])
  assert.equal(settled, true)
  assert.deepEqual(states, ['connecting', 'ended'])
})

test('a rejected session start closes its transport and ignores late ready', async (t) => {
  const sockets = installDeferredWebSockets(t)
  const states: string[] = []
  const ready = { count: 0 }
  const session = connectingSession(states, ready)

  const connecting = session.connect()
  assert.equal(sockets.length, 1)
  if (!sockets[0]) return
  let rejectStart: ((error: Error) => void) | null = null
  sockets[0].sendResult = new Promise((_, reject) => {
    rejectStart = reject
  })
  sockets[0].open()
  sockets[0].message({ type: 'session.ready' })
  rejectStart?.(new Error('session start failed'))

  await assert.rejects(connecting, /session start failed/)
  sockets[0].message({ type: 'session.ready' })

  assert.equal(sockets[0].closes, 1)
  assert.deepEqual(states, ['connecting', 'preparing', 'ended'])
  assert.equal(ready.count, 0)
})

test('session start declares protocol version and native build', () => {
  const event = createSessionStart('video')

  assert.equal(event.protocol_version, 5)
  assert.equal(event.client_build.length > 0, true)
  assert.equal(REALTIME_PROTOCOL_VERSION, 5)
  assert.equal('activation_mode' in event, false)
})

test('protocol v5 creates only strict audio and video mode-set events', () => {
  assert.deepEqual(createModeSet('video'), {
    type: 'session.mode.set',
    mode: 'video',
  })
  assert.throws(
    () => createModeSet('continuous_video' as never),
    /audio.*video|video.*audio/,
  )
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
  assert.deepEqual(harness.tools, [
    '',
    'remember 已完成',
    '\t 已完成',
  ])
})

test('a blank tool call id still completes the current tool state', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.started',
    response_id: 'response-1',
    name: 'remember',
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: ' ',
    name: 'remember',
  })

  assert.deepEqual(harness.results, [])
  assert.equal(harness.tools.at(-1), 'remember 已完成')
  assert.equal(harness.states.at(-1), 'thinking')
})

test('a duplicate tool completion still restores thinking without re-emitting its result', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: 'call-1',
    name: 'remember',
    result: { ok: true },
  })
  harness.receive({
    type: 'response.tool.started',
    response_id: 'response-1',
    name: 'remember',
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-1',
    call_id: 'call-1',
    name: 'remember',
    result: { ok: true },
  })

  assert.deepEqual(harness.results, [
    { callId: 'call-1', name: 'remember', result: { ok: true } },
  ])
  assert.equal(harness.tools.at(-1), 'remember 已完成')
  assert.equal(harness.states.at(-1), 'thinking')
})

test('invalid response ids neither replace nor complete the active response', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-current' })
  harness.receive({ type: 'response.created', response_id: 7 })
  harness.receive({
    type: 'response.tool.started',
    response_id: 'response-current',
    name: 'remember',
  })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 7,
    call_id: 'call-1',
    name: 'remember',
  })
  harness.receive({ type: 'response.done', response_id: 7 })
  harness.receive({ type: 'response.failed', response_id: ' ' })
  harness.receive({
    type: 'response.tool.completed',
    response_id: 'response-current',
    call_id: 'call-2',
    name: 'remember',
    result: { ok: true },
  })

  assert.deepEqual(harness.results, [
    { callId: 'call-2', name: 'remember', result: { ok: true } },
  ])
  assert.equal(harness.tools.at(-1), 'remember 已完成')
  assert.equal(harness.states.at(-1), 'thinking')
  assert.deepEqual(harness.responseFailures, [])
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

function readySessionHarness({
  mode = 'audio',
  onModeChanged = () => {},
  onFrameRequested = () => null,
  onFrameRequestState = () => {},
  modeChangeTimeoutMs,
}: {
  mode?: 'audio' | 'video'
  onModeChanged?: (mode: 'audio' | 'video') => void
  onFrameRequested?: () => string | null
  onFrameRequestState?: (active: boolean) => void
  modeChangeTimeoutMs?: number
} = {}) {
  const sent: Array<Record<string, unknown>> = []
  const errors: string[] = []
  const states: string[] = []
  const session = new RealtimeSession({
    server: '127.0.0.1:8700',
    accessToken: 'test-token',
    mode,
    onState: (state) => states.push(state),
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
    onFrameRequested,
    onFrameRequestState,
    onModeChanged,
    modeChangeTimeoutMs,
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
    states,
  }
}

test('setMode resolves only after a matching acknowledgement', async () => {
  const changedModes: string[] = []
  const { session, receive, sent } = readySessionHarness({
    onModeChanged: (mode) => changedModes.push(mode),
  })

  const changed = session.setMode('video')
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(sent.at(-1), { type: 'session.mode.set', mode: 'video' })

  receive({ type: 'session.mode.changed', mode: 'audio' })
  assert.equal(
    await Promise.race([
      changed.then(() => 'resolved'),
      new Promise((resolve) => setImmediate(() => resolve('waiting'))),
    ]),
    'waiting',
  )

  receive({ type: 'session.mode.changed', mode: 'video' })
  await changed
  assert.deepEqual(changedModes, ['audio', 'video'])
})

test('setMode coalesces the same target and rejects a conflicting target', async () => {
  const { session, receive, sent } = readySessionHarness()

  const first = session.setMode('video')
  const same = session.setMode('video')
  const conflicting = session.setMode('audio')
  await assert.rejects(conflicting, /切换.*进行|mode change.*pending/i)
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(
    sent.filter((event) => event.type === 'session.mode.set').length,
    1,
  )

  receive({ type: 'session.mode.changed', mode: 'video' })
  await Promise.all([first, same])
  const alreadyCurrent = session.setMode('video')
  await alreadyCurrent
  assert.equal(
    sent.filter((event) => event.type === 'session.mode.set').length,
    1,
  )
})

test('mode acknowledgement validation is own-property based and total', async () => {
  const changedModes: string[] = []
  const { session, receive } = readySessionHarness({
    onModeChanged: (mode) => changedModes.push(mode),
  })
  const changed = session.setMode('video')

  receive(Object.create({ type: 'session.mode.changed', mode: 'video' }))
  receive({ type: 'session.mode.changed', mode: 'camera' })
  assert.doesNotThrow(() => {
    ;(session as unknown as { handleEvent(event: unknown): void }).handleEvent(
      new Proxy({}, {
        getOwnPropertyDescriptor() {
          throw new Error('hostile proxy')
        },
      }),
    )
  })
  assert.equal(
    await Promise.race([
      changed.then(() => 'resolved'),
      new Promise((resolve) => setImmediate(() => resolve('waiting'))),
    ]),
    'waiting',
  )
  assert.deepEqual(changedModes, [])

  receive({ type: 'session.mode.changed', mode: 'video' })
  await changed
})

test('mode changes time out deterministically and late acknowledgements are inert', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const changedModes: string[] = []
  const { session, receive } = readySessionHarness({
    modeChangeTimeoutMs: 5_000,
    onModeChanged: (mode) => changedModes.push(mode),
  })
  const changed = session.setMode('video')
  t.mock.timers.tick(5_000)
  await assert.rejects(changed, /超时|timed out/i)

  receive({ type: 'session.mode.changed', mode: 'video' })
  assert.deepEqual(changedModes, ['video'])
  await session.setMode('video')
})

test('a timed-out mode becomes unknown and a corrective request still waits for ack', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const { session, receive, sent } = readySessionHarness({
    modeChangeTimeoutMs: 5_000,
  })
  const opening = session.setMode('video')
  t.mock.timers.tick(5_000)
  await assert.rejects(opening, /超时|timed out/i)

  const restoring = session.setMode('audio')
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(
    sent.filter((event) => event.type === 'session.mode.set'),
    [
      { type: 'session.mode.set', mode: 'video' },
      { type: 'session.mode.set', mode: 'audio' },
    ],
  )
  assert.equal(
    await Promise.race([
      restoring.then(() => 'resolved'),
      new Promise((resolve) => setImmediate(() => resolve('waiting'))),
    ]),
    'waiting',
  )
  receive({ type: 'session.mode.changed', mode: 'audio' })
  await restoring
})

test('old native messages and close callbacks cannot affect a replacement connection', async () => {
  const { session, receive, states } = readySessionHarness()
  const internals = session as unknown as {
    connectionGeneration: number
    handleTauriMessage(
      message: { type: 'Text'; data: string } | { type: 'Close' },
      generation: number,
    ): void
  }
  internals.connectionGeneration = 2
  const changed = session.setMode('video')

  internals.handleTauriMessage(
    {
      type: 'Text',
      data: JSON.stringify({ type: 'session.mode.changed', mode: 'video' }),
    },
    1,
  )
  internals.handleTauriMessage({ type: 'Close' }, 1)
  assert.equal(
    await Promise.race([
      changed.then(() => 'resolved'),
      new Promise((resolve) => setImmediate(() => resolve('waiting'))),
    ]),
    'waiting',
  )
  assert.deepEqual(states, [])

  internals.handleTauriMessage(
    {
      type: 'Text',
      data: JSON.stringify({ type: 'session.mode.changed', mode: 'video' }),
    },
    2,
  )
  await changed
  receive({ type: 'session.mode.changed', mode: 'video' })
})

test('transport replacement rejects pending mode work and closes the old transport', async () => {
  const { session } = readySessionHarness()
  const pending = session.setMode('video')
  let oldCloses = 0
  const internals = session as unknown as {
    transport: {
      send(message: string): Promise<void>
      close(): Promise<void>
    }
    connectionGeneration: number
    replaceTransport(
      transport: {
        send(message: string): Promise<void>
        close(): Promise<void>
      },
      generation: number,
    ): void
  }
  internals.transport = {
    send: async () => {},
    close: async () => {
      oldCloses += 1
    },
  }

  internals.replaceTransport(
    { send: async () => {}, close: async () => {} },
    internals.connectionGeneration,
  )

  await assert.rejects(pending, /替换|replaced/i)
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(oldCloses, 1)
})

test('an old in-flight send failure cannot close or replace the new transport', async () => {
  const { session, receive } = readySessionHarness()
  let rejectOld!: (error: Error) => void
  const oldPending = new Promise<void>((_resolve, reject) => {
    rejectOld = reject
  })
  const newSent: Array<Record<string, unknown>> = []
  const oldTransport = {
    send: async () => oldPending,
    close: async () => {},
  }
  const newTransport = {
    send: async (message: string) => newSent.push(JSON.parse(message)),
    close: async () => {},
  }
  const internals = session as unknown as {
    transport: typeof oldTransport
    connectionGeneration: number
    replaceTransport(transport: typeof newTransport, generation: number): void
  }
  internals.transport = oldTransport

  const oldSend = session.sendInput(new Float32Array([0.1]))
  await new Promise((resolve) => setImmediate(resolve))
  internals.replaceTransport(newTransport, internals.connectionGeneration)
  rejectOld(new Error('old transport failed'))
  await oldSend

  assert.equal(internals.transport, newTransport)
  const changed = session.setMode('video')
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(newSent, [{ type: 'session.mode.set', mode: 'video' }])
  receive({ type: 'session.mode.changed', mode: 'video' })
  await changed
})

test('a multi-event batch never splits across a replacement transport', async () => {
  const { session } = readySessionHarness()
  let releaseFirst!: () => void
  const firstPending = new Promise<void>((resolve) => {
    releaseFirst = resolve
  })
  const oldSent: Array<Record<string, unknown>> = []
  const newSent: Array<Record<string, unknown>> = []
  const oldTransport = {
    send: async (message: string) => {
      oldSent.push(JSON.parse(message))
      if (oldSent.length === 1) await firstPending
    },
    close: async () => {},
  }
  const newTransport = {
    send: async (message: string) => newSent.push(JSON.parse(message)),
    close: async () => {},
  }
  const internals = session as unknown as {
    transport: typeof oldTransport
    connectionGeneration: number
    replaceTransport(transport: typeof newTransport, generation: number): void
    sendEvents(events: Record<string, unknown>[]): Promise<void>
  }
  internals.transport = oldTransport
  const batch = internals.sendEvents([
    { type: 'test.first' },
    { type: 'test.second' },
  ])
  await new Promise((resolve) => setImmediate(resolve))

  internals.replaceTransport(newTransport, internals.connectionGeneration)
  releaseFirst()
  await assert.rejects(batch, /替换|旧连接|superseded/i)

  assert.deepEqual(oldSent, [{ type: 'test.first' }])
  assert.deepEqual(newSent, [])
})

test('an old successful send cannot settle or absorb a new transport queue item', async () => {
  const { session } = readySessionHarness()
  let releaseOld!: () => void
  const oldPending = new Promise<void>((resolve) => {
    releaseOld = resolve
  })
  const newSent: Array<Record<string, unknown>> = []
  const oldTransport = {
    send: async () => oldPending,
    close: async () => {},
  }
  const newTransport = {
    send: async (message: string) => newSent.push(JSON.parse(message)),
    close: async () => {},
  }
  const internals = session as unknown as {
    transport: typeof oldTransport
    connectionGeneration: number
    replaceTransport(transport: typeof newTransport, generation: number): void
    sendEvent(event: Record<string, unknown>): Promise<void>
  }
  internals.transport = oldTransport
  const oldSend = internals.sendEvent({ type: 'test.old' })
  await new Promise((resolve) => setImmediate(resolve))
  internals.replaceTransport(newTransport, internals.connectionGeneration)
  const newSend = internals.sendEvent({ type: 'test.new' })

  releaseOld()
  await assert.rejects(oldSend, /替换|旧连接|superseded/i)
  await newSend
  assert.deepEqual(newSent, [{ type: 'test.new' }])
})

test('close rejects a pending mode change and clears its timeout', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const { session, receive } = readySessionHarness({ modeChangeTimeoutMs: 5_000 })
  const changed = session.setMode('video')

  await session.close()
  await assert.rejects(changed, /关闭|closed/i)
  t.mock.timers.tick(5_000)
  receive({ type: 'session.mode.changed', mode: 'video' })
})

test('a correlated server error rejects only the pending mode change', async () => {
  const { session, receive } = readySessionHarness()
  const changed = session.setMode('video')

  receive({
    type: 'error',
    code: 'invalid_mode',
    mode: 'video',
    message: 'mode rejected',
  })

  await assert.rejects(changed, /mode rejected/)
})

test('unsafe and inherited error fields neither escape nor reject mode work', async () => {
  const { session } = readySessionHarness()
  const pending = session.setMode('video')
  const handleEvent = (event: unknown) =>
    (session as unknown as { handleEvent(event: unknown): void }).handleEvent(event)

  const accessorError = Object.create({
    code: 'invalid_mode',
    mode: 'video',
    message: 'inherited rejection',
  }) as Record<string, unknown>
  Object.defineProperty(accessorError, 'type', {
    value: 'error',
    enumerable: true,
  })
  Object.defineProperty(accessorError, 'response_id', {
    get() {
      throw new Error('response getter')
    },
  })
  Object.defineProperty(accessorError, 'message', {
    get() {
      throw new Error('message getter')
    },
  })
  assert.doesNotThrow(() => handleEvent(accessorError))

  const hostileError = new Proxy({}, {
    getOwnPropertyDescriptor(_target, key) {
      if (key === 'type') {
        return {
          configurable: true,
          enumerable: true,
          writable: true,
          value: 'error',
        }
      }
      throw new Error(`hostile ${String(key)}`)
    },
  })
  assert.doesNotThrow(() => handleEvent(hostileError))
  assert.equal(
    await Promise.race([
      pending.then(() => 'resolved', () => 'rejected'),
      new Promise((resolve) => setImmediate(() => resolve('waiting'))),
    ]),
    'waiting',
  )

  handleEvent({
    type: 'session.mode.changed',
    mode: 'video',
  })
  await pending
})

test('frame request state brackets capture and queueing without reentrant flicker', async () => {
  const states: boolean[] = []
  let session!: RealtimeSession
  let nested = false
  const harness = readySessionHarness({
    onFrameRequestState: (active) => states.push(active),
    onFrameRequested: () => {
      if (!nested) {
        nested = true
        harness.receive({
          type: 'input.frame.requested',
          response_id: 'response-nested',
        })
      }
      return 'jpeg-data'
    },
  })
  session = harness.session

  harness.receive({
    type: 'input.frame.requested',
    response_id: 'response-outer',
  })
  await new Promise((resolve) => setImmediate(resolve))

  assert.deepEqual(states, [true, false])
  assert.deepEqual(
    harness.sent.filter((event) => event.type === 'input.video.commit'),
    [
      { type: 'input.video.commit', response_id: 'response-nested' },
      { type: 'input.video.commit', response_id: 'response-outer' },
    ],
  )
  assert.equal(session instanceof RealtimeSession, true)
})

test('frame request state is restored when capture or callbacks throw', () => {
  const states: boolean[] = []
  const { receive, errors, sent } = readySessionHarness({
    onFrameRequestState: (active) => {
      states.push(active)
      if (active) throw new Error('state consumer failed')
    },
    onFrameRequested: () => {
      throw new Error('capture failed')
    },
  })

  assert.doesNotThrow(() => {
    receive({ type: 'input.frame.requested', response_id: 'response-1' })
  })
  assert.deepEqual(states, [true, false])
  assert.deepEqual(errors, ['capture failed'])
  assert.equal(sent.some((event) => event.type === 'input.video.commit'), false)
})

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
