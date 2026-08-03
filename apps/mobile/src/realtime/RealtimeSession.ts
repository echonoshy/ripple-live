import { Channel, invoke, isTauri } from '@tauri-apps/api/core'
import type { Message as TauriMessage } from '@tauri-apps/plugin-websocket'

export type RealtimeMode = 'audio' | 'video'
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
  name?: string
  result?: unknown
  response_id?: string
  artifact?: ResponseArtifact
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

type SessionOptions = {
  server: string
  accessToken: string
  conversationId?: string
  mode: RealtimeMode
  onState: (state: SessionState) => void
  onError: (message: string) => void
  onAssistantText: (text: string) => void
  onUserText: (text: string) => void
  onTool: (label: string) => void
  onAudio: (audio: Float32Array) => void
  onAudioDone: () => void
  onArtifact: (artifact: ResponseArtifact) => void
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
  private outputActive = false
  private interruptPending = false
  private currentResponseId: string | null = null
  private sendQueue: Promise<void> = Promise.resolve()

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
    await this.sendEvent({
        type: 'session.start',
        mode: this.options.mode,
    })
  }

  private sendEvent(event: Record<string, unknown>) {
    this.sendQueue = this.sendQueue.then(async () => {
      if (!this.transport || this.closed) return
      await this.transport.send(JSON.stringify(event))
    })
    return this.sendQueue
  }

  private handleTauriMessage(message: TauriMessage) {
    if (message.type === 'Text') {
      this.handleText(message.data)
    } else if (message.type === 'Close' && !this.closed) {
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
      case 'input.transcript.final':
        this.options.onUserText(event.text?.trim() ?? '')
        break
      case 'response.created':
        this.currentResponseId = event.response_id ?? null
        this.assistantText = ''
        this.outputActive = false
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
        this.outputActive = true
        this.assistantText += event.delta
        this.options.onAssistantText(this.assistantText)
        this.options.onState('speaking')
        break
      case 'response.audio.delta':
        if (!this.isCurrentResponse(event)) return
        if (this.interruptPending || !event.audio) return
        this.outputActive = true
        this.options.onAudio(base64ToFloat32(event.audio))
        this.options.onState('speaking')
        break
      case 'ripple.response.artifact.added':
        if (!this.isCurrentResponse(event) || !event.artifact) return
        this.options.onArtifact(event.artifact)
        break
      case 'response.done':
        if (!this.isCurrentResponse(event)) return
        this.options.onAudioDone()
        this.currentResponseId = null
        this.outputActive = false
        this.interruptPending = false
        this.options.onTool('')
        this.options.onState('listening')
        break
      case 'response.cancelled':
        if (!this.isCurrentResponse(event)) return
        this.currentResponseId = null
        this.outputActive = false
        this.interruptPending = false
        this.options.onTool('')
        this.options.onState('listening')
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

  async speechStarted() {
    if (!this.transport || !this.ready || this.closed) return
    const hadOutput = this.outputActive
    this.outputActive = false
    this.currentResponseId = null
    this.interruptPending = hadOutput
    this.assistantText = ''
    this.options.onAssistantText('')
    this.options.onTool('')
    this.options.onState('listening')
    await this.sendEvent({ type: 'input.speech_started' })
  }

  async sendInput(audio: Float32Array, frame: string | null) {
    if (!this.transport || !this.ready || this.closed) return
    try {
      await this.sendEvent({
        type: 'input.audio.append',
        audio: float32ToBase64(audio),
        sample_rate: 16000,
      })
      if (this.options.mode === 'video' && frame) {
        await this.sendEvent({
          type: 'input.video.frame',
          image: frame,
          mime_type: 'image/jpeg',
          captured_at: Date.now(),
        })
      }
    } catch (error) {
      this.ready = false
      this.closed = true
      this.options.onError(
        error instanceof Error ? error.message : '实时音视频发送失败',
      )
      try {
        await this.transport.close()
      } catch {
        // The socket may already be gone.
      }
    }
  }

  async commitInput() {
    if (!this.transport || !this.ready || this.closed) return
    await this.sendEvent({ type: 'input.commit' })
  }

  forceListen() {
    if (!this.transport || this.closed) return false
    this.outputActive = false
    this.currentResponseId = null
    this.interruptPending = true
    this.options.onTool('')
    this.options.onState('listening')
    void this.sendEvent({ type: 'response.cancel' })
    return true
  }

  async close() {
    if (this.closed) return
    this.closed = true
    if (this.transport) {
      try {
        await this.sendQueue.catch(() => {})
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
