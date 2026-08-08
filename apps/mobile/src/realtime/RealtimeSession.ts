import { Channel, invoke, isTauri } from '@tauri-apps/api/core'
import type { Message as TauriMessage } from '@tauri-apps/plugin-websocket'
import {
  createModeSet,
  createRequestedFrameEvents,
  createSessionStart,
  createTurnId,
} from './protocol'
import type { RealtimeMode } from './protocol'
import type { ToolCompletion } from './toolResults'

export type { RealtimeMode } from './protocol'
export type SessionState =
  | 'idle'
  | 'connecting'
  | 'preparing'
  | 'listening'
  | 'thinking'
  | 'using_tool'
  | 'speaking'
  | 'ended'
  | 'error'

type RealtimeEvent = {
  type: string
  session_id?: string
  conversation_id?: string
  text?: string
  delta?: string
  audio?: string
  sample_rate?: number
  message?: string
  code?: string
  name?: string
  call_id?: string
  result?: unknown
  response_id?: string
  artifact?: ResponseArtifact
  reason?: string
  needs_frame?: boolean
  turn_id?: string
  decision?: 'complete' | 'continue' | 'uncertain'
  command?: string
  mode?: RealtimeMode
}

export type ResponseArtifact = {
  id: string
  kind: 'image' | string
  memory_id?: string
  caption: string
  content_url: string
}

type Transport = {
  send(message: string): Promise<void>
  close(): Promise<void>
}

type SendPriority = 'normal' | 'high'

type QueuedSend = {
  messages: string[]
  resolve: () => void
  reject: (error: unknown) => void
  onFailure?: (error: unknown) => void
}

export type SessionOptions = {
  server: string
  accessToken: string
  conversationId?: string
  mode: RealtimeMode
  onState: (state: SessionState) => void
  onError: (message: string) => void
  onResponseFailed: (message: string) => void
  onAssistantText: (text: string) => void
  onUserText: (text: string) => void
  onTool: (label: string) => void
  onToolResult: (event: ToolCompletion) => void
  onAudio: (audio: Float32Array) => void
  onAudioDone: () => void
  onInterrupted: () => void
  onArtifact: (artifact: ResponseArtifact) => void
  onFrameRequested: () => string | null
  onFrameRequestState?: (active: boolean) => void
  onModeChanged?: (mode: RealtimeMode) => void
  modeChangeTimeoutMs?: number
  onReady: () => Promise<void>
  onConversation: (conversationId: string) => void
}

type PendingModeChange = {
  mode: RealtimeMode
  promise: Promise<void>
  resolve: () => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

function ownValue(value: unknown, key: string): unknown {
  if ((typeof value !== 'object' && typeof value !== 'function') || value === null) {
    return undefined
  }
  try {
    const descriptor = Object.getOwnPropertyDescriptor(value, key)
    return descriptor && 'value' in descriptor ? descriptor.value : undefined
  } catch {
    return undefined
  }
}

function ownString(value: unknown, key: string): string | undefined {
  const candidate = ownValue(value, key)
  return typeof candidate === 'string' ? candidate : undefined
}

function ownMode(value: unknown): RealtimeMode | undefined {
  const mode = ownString(value, 'mode')
  return mode === 'audio' || mode === 'video' ? mode : undefined
}

function float32ToBase64(samples: Float32Array) {
  const bytes = new Uint8Array(
    samples.buffer,
    samples.byteOffset,
    samples.byteLength,
  )
  let binary = ''
  const block = 0x8000
  for (let offset = 0; offset < bytes.length; offset += block) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + block))
  }
  return btoa(binary)
}

function base64ToFloat32(encoded: string) {
  const binary = atob(encoded)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return new Float32Array(bytes.buffer).slice()
}

function normalizeServer(server: string) {
  return server
    .trim()
    .replace(/^wss?:\/\//, '')
    .replace(/\/+$/, '')
}

function isNonBlankString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0
}

async function connectTauriWebSocket(
  url: string,
  onMessage: (message: TauriMessage) => void,
): Promise<{ transport: Transport; activate: () => void }> {
  const pending: TauriMessage[] = []
  let ready = false
  const channel = new Channel<TauriMessage>()
  channel.onmessage = (message) => {
    if (ready) onMessage(message)
    else pending.push(message)
  }

  const id = await invoke<number>('plugin:websocket|connect', {
    url,
    onMessage: channel,
    config: {
      maxMessageSize: 128 * 1024 * 1024,
      maxFrameSize: 128 * 1024 * 1024,
      writeBufferSize: 0,
    },
  })

  return {
    transport: {
      send: (message) =>
        invoke('plugin:websocket|send', {
          id,
          message: { type: 'Text', data: message },
        }),
      close: () =>
        invoke('plugin:websocket|send', {
          id,
          message: {
            type: 'Close',
            data: { code: 1000, reason: 'Disconnected by client' },
          },
        }),
    },
    activate: () => {
      ready = true
      pending.splice(0).forEach(onMessage)
    },
  }
}

export class RealtimeSession {
  private readonly options: SessionOptions
  private conversationId: string | null
  private transport: Transport | null = null
  private ready = false
  private closed = false
  private connectionGeneration = 0
  private assistantText = ''
  private interruptPending = false
  private currentResponseId: string | null = null
  private completedToolCallIds = new Set<string>()
  private playbackActive = false
  private playbackStartedReported = false
  private normalSends: QueuedSend[] = []
  private highPrioritySends: QueuedSend[] = []
  private sending = false
  private sendIdleWaiters: Array<() => void> = []
  private currentTurnId: string | null = null
  private pendingTurnId: string | null = null
  private endpointTimer: ReturnType<typeof setTimeout> | null = null
  private inputClearBarrier: Promise<void> | null = null
  private currentMode: RealtimeMode
  private pendingModeChange: PendingModeChange | null = null
  private frameRequestsActive = 0

  constructor(options: SessionOptions) {
    this.options = options
    this.currentMode = options.mode
    this.conversationId = isNonBlankString(options.conversationId)
      ? options.conversationId
      : null
  }

  async connect() {
    if (this.closed) return
    const generation = ++this.connectionGeneration
    const params = new URLSearchParams({
      mode: this.options.mode,
      access_token: this.options.accessToken,
    })
    if (this.conversationId) params.set('conversation_id', this.conversationId)
    const url = `ws://${normalizeServer(this.options.server)}/v1/agent/realtime?${params}`
    console.info('[Ripple Live] connecting session', {
      conversationId: this.conversationId,
      mode: this.options.mode,
    })
    this.options.onState('connecting')

    try {
      if (isTauri()) {
        const connection = await connectTauriWebSocket(
          url,
          (message) => this.handleTauriMessage(message),
        )
        if (!this.isActiveConnection(generation)) {
          await connection.transport.close().catch(() => {})
          return
        }
        this.replaceTransport(connection.transport)
        await this.startSession(generation)
        if (!this.isActiveConnection(generation)) return
        connection.activate()
        return
      }

      await new Promise<void>((resolve, reject) => {
        const socket = new WebSocket(url)
        const pendingMessages: string[] = []
        let activated = false
        socket.onopen = () => {
          const transport: Transport = {
            send: async (message) => socket.send(message),
            close: async () => socket.close(1000, 'user_stop'),
          }
          if (!this.isActiveConnection(generation)) {
            void transport.close().then(resolve).catch(resolve)
            return
          }
          this.replaceTransport(transport)
          void this.startSession(generation)
            .then(() => {
              if (!this.isActiveConnection(generation)) {
                resolve()
                return
              }
              activated = true
              pendingMessages.splice(0).forEach((message) => {
                this.handleText(message)
              })
              resolve()
            })
            .catch(reject)
        }
        socket.onmessage = (event) => {
          if (this.isActiveConnection(generation)) {
            const message = String(event.data)
            if (activated) this.handleText(message)
            else pendingMessages.push(message)
          }
        }
        socket.onerror = () => {
          if (this.isActiveConnection(generation)) {
            const error = new Error(`无法连接 ${url}`)
            this.rejectPendingModeChange(error)
            reject(error)
          }
          else resolve()
        }
        socket.onclose = () => {
          if (this.isActiveConnection(generation)) {
            this.clearEndpointState()
            this.rejectPendingModeChange(new Error('实时会话已关闭'))
            this.closed = true
            this.connectionGeneration += 1
            this.ready = false
            this.options.onState('ended')
          } else {
            resolve()
          }
        }
      })
    } catch (error) {
      if (this.isActiveConnection(generation)) await this.close()
      throw error
    }
  }

  private isActiveConnection(generation: number) {
    return !this.closed && generation === this.connectionGeneration
  }

  private replaceTransport(transport: Transport) {
    if (this.transport && this.transport !== transport) {
      this.rejectPendingModeChange(new Error('实时连接已被替换'))
    }
    this.transport = transport
  }

  private async startSession(generation: number) {
    if (!this.transport || !this.isActiveConnection(generation)) return
    this.options.onState('preparing')
    await this.sendEvent(
      createSessionStart(this.options.mode),
    )
  }

  private sendEvent(
    event: Record<string, unknown>,
    priority: SendPriority = 'normal',
    onFailure?: (error: unknown) => void,
  ) {
    return this.sendEvents([event], priority, onFailure)
  }

  private sendEvents(
    events: Record<string, unknown>[],
    priority: SendPriority = 'normal',
    onFailure?: (error: unknown) => void,
  ) {
    if (!this.transport || this.closed) return Promise.resolve()
    return new Promise<void>((resolve, reject) => {
      const queue = priority === 'high' ? this.highPrioritySends : this.normalSends
      queue.push({
        messages: events.map((event) => JSON.stringify(event)),
        resolve,
        reject,
        onFailure,
      })
      void this.drainSendQueue()
    })
  }

  private async drainSendQueue() {
    if (this.sending) return
    this.sending = true
    try {
      while (this.highPrioritySends.length || this.normalSends.length) {
        const item = this.highPrioritySends.shift() ?? this.normalSends.shift()
        if (!item) continue
        try {
          if (this.transport) {
            for (const message of item.messages) {
              await this.transport.send(message)
            }
          }
          item.resolve()
        } catch (error) {
          item.onFailure?.(error)
          item.reject(error)
          if (item.onFailure) {
            this.rejectQueuedSends(error)
            return
          }
        }
      }
    } finally {
      this.sending = false
      if (!this.highPrioritySends.length && !this.normalSends.length) {
        this.sendIdleWaiters.splice(0).forEach((resolve) => resolve())
      } else {
        void this.drainSendQueue()
      }
    }
  }

  private rejectQueuedSends(error: unknown) {
    const queued = [...this.highPrioritySends, ...this.normalSends]
    this.highPrioritySends = []
    this.normalSends = []
    queued.forEach((item) => item.reject(error))
  }

  private waitForSendIdle() {
    if (
      !this.sending &&
      !this.highPrioritySends.length &&
      !this.normalSends.length
    ) {
      return Promise.resolve()
    }
    return new Promise<void>((resolve) => this.sendIdleWaiters.push(resolve))
  }

  private handleTauriMessage(message: TauriMessage) {
    if (this.closed) return
    if (message.type === 'Text') {
      this.handleText(message.data)
    } else if (message.type === 'Close' && !this.closed) {
      this.clearEndpointState()
      this.rejectPendingModeChange(new Error('实时会话已关闭'))
      this.closed = true
      this.ready = false
      this.options.onState('ended')
    }
  }

  private handleText(text: string) {
    if (this.closed) return
    let event: unknown
    try {
      event = JSON.parse(text) as unknown
    } catch {
      return
    }
    this.handleEvent(event)
  }

  private handleEvent(value: unknown) {
    if (this.closed) return
    const type = ownString(value, 'type')
    if (!type) return

    if (type === 'session.mode.changed') {
      const mode = ownMode(value)
      if (!mode) return
      this.currentMode = mode
      const pending = this.pendingModeChange
      if (pending?.mode === mode) {
        this.pendingModeChange = null
        clearTimeout(pending.timer)
        pending.resolve()
      }
      try {
        this.options.onModeChanged?.(mode)
      } catch {
        // UI callbacks cannot interrupt realtime event processing.
      }
      return
    }

    if (type === 'error') {
      this.rejectModeChangeForServerError(value)
    }

    const event = value as RealtimeEvent

    switch (type) {
      case 'session.created':
        if (isNonBlankString(event.conversation_id)) {
          this.conversationId = event.conversation_id
          this.options.onConversation(this.conversationId)
        } else if (
          event.conversation_id === undefined &&
          isNonBlankString(event.session_id)
        ) {
          this.conversationId = event.session_id
          this.options.onConversation(this.conversationId)
        } else if (
          event.conversation_id === undefined &&
          event.session_id === undefined &&
          this.conversationId
        ) {
          this.options.onConversation(this.conversationId)
        }
        console.info('[Ripple Live] session created', {
          conversationId: this.conversationId,
        })
        break
      case 'session.ready':
        console.info('[Ripple Live] session ready', {
          conversationId: event.session_id ?? this.conversationId,
        })
        this.ready = true
        this.options.onState('listening')
        void this.options.onReady().catch((error: unknown) => {
          const message =
            error instanceof Error ? error.message : '无法启动麦克风或摄像头'
          this.options.onError(message)
        })
        break
      case 'input.speech_started':
        this.options.onState('listening')
        break
      case 'input.turn.decision':
        this.handleTurnDecision(event)
        break
      case 'input.command.handled':
        if (!this.matchesEndpointTurn(event.turn_id)) return
        this.clearEndpointState()
        this.options.onState('listening')
        break
      case 'input.frame.requested': {
        const responseId = event.response_id
        if (!isNonBlankString(responseId)) {
          this.options.onError('服务端没有提供画面请求标识')
          break
        }
        this.beginFrameRequest()
        try {
          const frame = this.options.onFrameRequested()
          void this.sendEvents(
            createRequestedFrameEvents(responseId, frame, Date.now()),
            'high',
          )
        } catch (error) {
          this.options.onError(
            error instanceof Error ? error.message : '无法捕获请求的画面',
          )
        } finally {
          this.endFrameRequest()
        }
        break
      }
      case 'input.transcript.final':
        this.options.onUserText(event.text?.trim() ?? '')
        break
      case 'response.created':
        if (!isNonBlankString(event.response_id)) return
        this.currentResponseId = event.response_id
        this.completedToolCallIds.clear()
        this.playbackStartedReported = false
        this.assistantText = ''
        this.interruptPending = false
        this.options.onTool('')
        this.options.onState('thinking')
        break
      case 'response.tool.started':
        if (!this.isCurrentResponse(event)) return
        this.options.onTool(event.name ? `正在调用 ${event.name}` : '正在调用工具')
        this.options.onState('using_tool')
        break
      case 'response.tool.completed':
        if (!this.isCurrentResponse(event)) return
        this.options.onTool(event.name ? `${event.name} 已完成` : '工具调用已完成')
        this.options.onState('thinking')
        this.emitToolResult(event)
        break
      case 'response.text.delta':
        if (!this.isCurrentResponse(event)) return
        if (this.interruptPending || !event.delta) return
        this.assistantText += event.delta
        this.options.onAssistantText(this.assistantText)
        this.options.onState('speaking')
        break
      case 'response.audio.delta':
        if (!this.isCurrentResponse(event)) return
        if (this.interruptPending || !event.audio) return
        this.playbackActive = true
        this.options.onAudio(base64ToFloat32(event.audio))
        this.options.onState('speaking')
        break
      case 'ripple.response.artifact.added':
        if (!this.isCurrentResponse(event) || !event.artifact) return
        this.options.onArtifact(event.artifact)
        break
      case 'response.done':
        if (!this.isCurrentResponse(event)) return
        this.finishResponse('done')
        break
      case 'response.cancelled':
        if (!this.isCurrentResponse(event)) return
        this.finishResponse('cancelled')
        break
      case 'response.failed':
        if (!this.isCurrentResponse(event)) return
        this.finishResponse(
          'failed',
          event.message ?? '本次处理失败，请重试',
        )
        break
      case 'error':
        console.error('[Ripple Live] session error', {
          conversationId: this.conversationId,
          responseId: event.response_id,
          message: event.message,
        })
        this.options.onError(event.message ?? 'Agent 服务返回错误')
        break
    }
  }

  private beginFrameRequest() {
    this.frameRequestsActive += 1
    if (this.frameRequestsActive !== 1) return
    try {
      this.options.onFrameRequestState?.(true)
    } catch {
      // UI feedback cannot prevent frame capture.
    }
  }

  private endFrameRequest() {
    this.frameRequestsActive = Math.max(0, this.frameRequestsActive - 1)
    if (this.frameRequestsActive !== 0) return
    try {
      this.options.onFrameRequestState?.(false)
    } catch {
      // UI feedback cannot interrupt realtime event processing.
    }
  }

  private rejectModeChangeForServerError(value: unknown) {
    const pending = this.pendingModeChange
    if (!pending) return
    const responseId = ownString(value, 'response_id')
    const mode = ownString(value, 'mode')
    const code = ownString(value, 'code')
    const correlated = mode === pending.mode
    const modeError = code === 'invalid_mode' || code === 'unsupported_protocol'
    const sessionError = responseId === undefined && mode === undefined
    if (!correlated && !(modeError && mode === undefined) && !sessionError) return
    const message = ownString(value, 'message') ?? '服务端拒绝切换会话模式'
    this.rejectPendingModeChange(new Error(message))
  }

  private rejectPendingModeChange(error: Error) {
    const pending = this.pendingModeChange
    if (!pending) return
    this.pendingModeChange = null
    clearTimeout(pending.timer)
    pending.reject(error)
  }

  private isCurrentResponse(event: RealtimeEvent) {
    return (
      this.currentResponseId !== null &&
      isNonBlankString(event.response_id) &&
      event.response_id === this.currentResponseId
    )
  }

  private emitToolResult(event: RealtimeEvent) {
    if (
      !this.currentResponseId ||
      event.response_id !== this.currentResponseId ||
      typeof event.call_id !== 'string' ||
      !event.call_id.trim() ||
      typeof event.name !== 'string' ||
      !event.name.trim() ||
      this.completedToolCallIds.has(event.call_id)
    ) {
      return
    }

    this.completedToolCallIds.add(event.call_id)
    try {
      this.options.onToolResult({
        callId: event.call_id,
        name: event.name,
        result: event.result,
      })
    } catch {
      // Consumer rendering must not interrupt the realtime transport.
    }
  }

  private matchesEndpointTurn(turnId: string | undefined) {
    return !!turnId && (turnId === this.pendingTurnId || turnId === this.currentTurnId)
  }

  private handleTurnDecision(event: RealtimeEvent) {
    const turnId = event.turn_id
    if (!turnId || turnId !== this.pendingTurnId || turnId !== this.currentTurnId) {
      return
    }
    if (event.decision === 'complete') {
      this.commitPendingTurn(turnId, false)
    } else if (event.decision === 'continue' || event.decision === 'uncertain') {
      this.clearEndpointTimer()
      this.endpointTimer = setTimeout(() => {
        this.commitPendingTurn(turnId, true)
      }, 1_500)
    }
  }

  private clearEndpointTimer() {
    if (this.endpointTimer === null) return
    clearTimeout(this.endpointTimer)
    this.endpointTimer = null
  }

  private clearEndpointState() {
    this.clearEndpointTimer()
    this.currentTurnId = null
    this.pendingTurnId = null
  }

  private commitPendingTurn(turnId: string, endpointFallback: boolean) {
    if (turnId !== this.pendingTurnId) return
    this.clearEndpointTimer()
    this.pendingTurnId = null
    this.currentTurnId = null
    this.sendEndpointEvent({
      type: 'input.commit',
      turn_id: turnId,
      endpoint_fallback: endpointFallback,
    })
  }

  private sendEndpointEvent(event: Record<string, unknown>) {
    void this.sendEvent(event, 'normal', (error: unknown) => {
      void this.handleSendFailure(error)
    }).catch(() => {})
  }

  private scheduleInputClear() {
    let failure: Promise<void> | null = null
    const clear = this.sendEvent({ type: 'input.clear' }, 'normal', (error) => {
      failure = this.handleSendFailure(error)
    })
    const barrier = clear.catch(async () => {
      await failure
    })
    this.inputClearBarrier = barrier
    void barrier.then(() => {
      if (this.inputClearBarrier === barrier) this.inputClearBarrier = null
    })
  }

  private async waitForInputClear() {
    while (this.inputClearBarrier) await this.inputClearBarrier
  }

  private async handleSendFailure(error: unknown) {
    if (this.closed) return
    this.clearEndpointState()
    this.rejectPendingModeChange(
      error instanceof Error ? error : new Error('实时音视频发送失败'),
    )
    this.ready = false
    this.closed = true
    this.options.onError(
      error instanceof Error ? error.message : '实时音视频发送失败',
    )
    try {
      await this.transport?.close()
    } catch {
      // The socket may already be gone.
    }
  }

  private finishResponse(
    outcome: 'done' | 'cancelled' | 'failed',
    message?: string,
  ) {
    if (outcome === 'done') {
      this.options.onAudioDone()
    } else {
      this.options.onInterrupted()
      this.assistantText = ''
      this.options.onAssistantText('')
    }
    this.currentResponseId = null
    this.completedToolCallIds.clear()
    this.playbackStartedReported = false
    this.interruptPending = false
    this.options.onTool('')
    this.options.onState('listening')
    if (outcome === 'failed') {
      this.options.onResponseFailed(message ?? '本次处理失败，请重试')
    }
  }

  outputPlaybackStarted(bufferedMs: number) {
    if (!this.currentResponseId || this.playbackStartedReported) return
    this.playbackActive = true
    this.playbackStartedReported = true
    void this.sendEvent({
      type: 'output.playback.started',
      response_id: this.currentResponseId,
      buffered_ms: bufferedMs,
    })
  }

  async speechStarted() {
    if (this.inputClearBarrier) await this.waitForInputClear()
    if (!this.transport || !this.ready || this.closed) return
    if (this.pendingTurnId) {
      const turnId = this.pendingTurnId
      this.clearEndpointTimer()
      this.pendingTurnId = null
      this.options.onState('listening')
      await this.sendEvent({ type: 'input.speech_resumed', turn_id: turnId })
      return
    }
    if (this.currentResponseId || this.playbackActive) {
      this.currentResponseId = null
      this.interruptPending = true
      this.playbackActive = false
      this.assistantText = ''
      this.playbackStartedReported = false
      this.options.onInterrupted()
      this.options.onAssistantText('')
      this.options.onTool('')
      await this.sendEvent({ type: 'response.cancel' }, 'high')
    }
    if (this.inputClearBarrier) await this.waitForInputClear()
    if (!this.transport || !this.ready || this.closed) return
    this.currentTurnId = createTurnId()
    this.options.onState('listening')
    await this.sendEvent(
      { type: 'input.speech_started', turn_id: this.currentTurnId },
      'high',
    )
  }

  outputPlaybackEnded() {
    this.playbackActive = false
  }

  async sendInput(audio: Float32Array) {
    if (!this.transport || !this.ready || this.closed) return
    try {
      await this.sendEvent({
        type: 'input.audio.append',
        audio: float32ToBase64(audio),
        sample_rate: 16000,
      })
    } catch (error) {
      await this.handleSendFailure(error)
    }
  }

  speechPaused() {
    if (!this.transport || !this.ready || this.closed || !this.currentTurnId) return
    this.clearEndpointTimer()
    this.pendingTurnId = this.currentTurnId
    this.sendEndpointEvent({
      type: 'input.turn.pause',
      turn_id: this.currentTurnId,
    })
  }

  discardInput() {
    this.clearEndpointState()
    if (!this.transport || !this.ready || this.closed) return
    void this.sendEvent({ type: 'input.clear' })
  }

  forceListen() {
    this.clearEndpointState()
    if (!this.transport || this.closed) return false
    const hasActiveOutput = this.currentResponseId !== null || this.playbackActive
    this.currentResponseId = null
    this.interruptPending = true
    this.playbackActive = false
    this.options.onTool('')
    this.options.onState('listening')
    void this.sendEvent({ type: 'response.cancel', clear_input: true }, 'high')
    this.scheduleInputClear()
    return hasActiveOutput
  }

  setMode(mode: RealtimeMode): Promise<void> {
    if (mode !== 'audio' && mode !== 'video') {
      return Promise.reject(new TypeError('会话模式只支持 audio 或 video'))
    }
    const pending = this.pendingModeChange
    if (pending) {
      if (pending.mode === mode) return pending.promise
      return Promise.reject(new Error('另一个模式切换正在进行'))
    }
    if (this.currentMode === mode) return Promise.resolve()
    if (!this.transport || !this.ready || this.closed) {
      return Promise.reject(new Error('实时会话尚未就绪或已关闭'))
    }

    let resolvePending!: () => void
    let rejectPending!: (error: Error) => void
    const promise = new Promise<void>((resolve, reject) => {
      resolvePending = resolve
      rejectPending = reject
    })
    const timeoutMs = this.options.modeChangeTimeoutMs ?? 5_000
    const timer = setTimeout(() => {
      this.rejectPendingModeChange(new Error('切换会话模式超时'))
    }, timeoutMs)
    this.pendingModeChange = {
      mode,
      promise,
      resolve: resolvePending,
      reject: rejectPending,
      timer,
    }
    void this.sendEvent(createModeSet(mode), 'high', (error) => {
      const failure =
        error instanceof Error ? error : new Error('发送模式切换请求失败')
      this.rejectPendingModeChange(failure)
      void this.handleSendFailure(failure)
    }).catch(() => {})
    return promise
  }

  async close() {
    this.clearEndpointState()
    this.rejectPendingModeChange(new Error('实时会话已关闭'))
    this.connectionGeneration += 1
    if (this.closed) return
    this.closed = true
    const transport = this.transport
    this.transport = null
    if (transport) {
      try {
        await this.waitForSendIdle()
        await transport.send(JSON.stringify({ type: 'session.close' }))
      } catch {
        // A disconnected socket still counts as closed.
      }
      try {
        await transport.close()
      } catch {
        // Native disconnect can fail when the peer has already closed.
      }
    }
    console.info('[Ripple Live] session closed', {
      conversationId: this.conversationId,
    })
    this.options.onState('ended')
  }
}
