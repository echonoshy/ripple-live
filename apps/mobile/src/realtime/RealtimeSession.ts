import { Channel, invoke, isTauri } from '@tauri-apps/api/core'
import type { Message as TauriMessage } from '@tauri-apps/plugin-websocket'
import {
  createRequestedFrameEvents,
  createSessionStart,
  createTurnId,
} from './protocol'
import type { RealtimeMode } from './protocol'

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
  result?: unknown
  response_id?: string
  artifact?: ResponseArtifact
  reason?: string
  needs_frame?: boolean
  turn_id?: string
  decision?: 'complete' | 'continue' | 'uncertain'
  command?: string
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

type SessionOptions = {
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
  onAudio: (audio: Float32Array) => void
  onAudioDone: () => void
  onInterrupted: () => void
  onArtifact: (artifact: ResponseArtifact) => void
  onFrameRequested: () => string | null
  onReady: () => Promise<void>
  onConversation: (conversationId: string) => void
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
  private assistantText = ''
  private interruptPending = false
  private currentResponseId: string | null = null
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

  constructor(options: SessionOptions) {
    this.options = options
    this.conversationId = options.conversationId ?? null
  }

  async connect() {
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

    if (isTauri()) {
      const connection = await connectTauriWebSocket(
        url,
        (message) => this.handleTauriMessage(message),
      )
      this.transport = connection.transport
      connection.activate()
      await this.startSession()
      return
    }

    await new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(url)
      socket.onopen = () => {
        this.transport = {
          send: async (message) => socket.send(message),
          close: async () => socket.close(1000, 'user_stop'),
        }
        void this.startSession().then(resolve).catch(reject)
      }
      socket.onmessage = (event) => this.handleText(String(event.data))
      socket.onerror = () => reject(new Error(`无法连接 ${url}`))
      socket.onclose = () => {
        if (!this.closed) {
          this.clearEndpointState()
          this.closed = true
          this.ready = false
          this.options.onState('ended')
        }
      }
    })
  }

  private async startSession() {
    if (!this.transport) return
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
    if (message.type === 'Text') {
      this.handleText(message.data)
    } else if (message.type === 'Close' && !this.closed) {
      this.clearEndpointState()
      this.closed = true
      this.ready = false
      this.options.onState('ended')
    }
  }

  private handleText(text: string) {
    let event: RealtimeEvent
    try {
      event = JSON.parse(text) as RealtimeEvent
    } catch {
      return
    }

    switch (event.type) {
      case 'session.created':
        this.conversationId =
          event.conversation_id ?? event.session_id ?? this.conversationId
        if (this.conversationId) this.options.onConversation(this.conversationId)
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
        const frame = this.options.onFrameRequested()
        const responseId = event.response_id
        if (!responseId) {
          this.options.onError('服务端没有提供画面请求标识')
          break
        }
        void this.sendEvents(
          createRequestedFrameEvents(responseId, frame, Date.now()),
          'high',
        )
        break
      }
      case 'input.transcript.final':
        this.options.onUserText(event.text?.trim() ?? '')
        break
      case 'response.created':
        this.currentResponseId = event.response_id ?? null
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

  private isCurrentResponse(event: RealtimeEvent) {
    return !event.response_id || event.response_id === this.currentResponseId
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
    void this.sendEvent({ type: 'response.cancel' }, 'high')
    this.scheduleInputClear()
    return hasActiveOutput
  }

  async close() {
    this.clearEndpointState()
    if (this.closed) return
    this.closed = true
    if (this.transport) {
      try {
        await this.waitForSendIdle()
        await this.transport.send(JSON.stringify({ type: 'session.close' }))
      } catch {
        // A disconnected socket still counts as closed.
      }
      try {
        await this.transport.close()
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
