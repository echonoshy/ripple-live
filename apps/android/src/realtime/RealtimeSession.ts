import { Channel, invoke, isTauri } from '@tauri-apps/api/core'
import type { Message as TauriMessage } from '@tauri-apps/plugin-websocket'

export type RealtimeMode = 'audio' | 'video'
export type SessionState =
  | 'idle'
  | 'connecting'
  | 'queued'
  | 'preparing'
  | 'listening'
  | 'speaking'
  | 'paused'
  | 'ended'
  | 'error'

type RealtimeEvent = {
  type: string
  kind?: 'listen' | 'text' | 'audio'
  text?: string
  audio?: string
  position?: number
  reason?: string
  error?: {
    message?: string
    code?: string
  }
}

type Transport = {
  send(message: string): Promise<void>
  close(): Promise<void>
}

type SessionOptions = {
  server: string
  mode: RealtimeMode
  onState: (state: SessionState) => void
  onError: (message: string) => void
  onAssistantText: (text: string) => void
  onUserText: (text: string) => void
  onAudio: (audio: Float32Array) => void
  onReady: () => Promise<void>
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
  const aligned = bytes.byteOffset % 4 === 0 ? bytes : bytes.slice()
  return new Float32Array(
    aligned.buffer,
    aligned.byteOffset,
    Math.floor(aligned.byteLength / 4),
  ).slice()
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

  const transport: Transport = {
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
  }

  return {
    transport,
    activate: () => {
      ready = true
      pending.splice(0).forEach(onMessage)
    },
  }
}

export class RealtimeSession {
  private readonly options: SessionOptions
  private transport: Transport | null = null
  private initialized = false
  private ready = false
  private closed = false
  private assistantText = ''
  private userText = ''
  private forceListenNext = false

  constructor(options: SessionOptions) {
    this.options = options
  }

  async connect() {
    const url = `ws://${normalizeServer(this.options.server)}/v1/realtime?mode=${this.options.mode}`
    this.options.onState('connecting')

    if (isTauri()) {
      const connection = await connectTauriWebSocket(
        url,
        (message) => this.handleTauriMessage(message),
      )
      this.transport = connection.transport
      connection.activate()
      return
    }

    await new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(url)
      socket.onopen = () => {
        this.transport = {
          send: async (message) => socket.send(message),
          close: async () => socket.close(1000, 'user_stop'),
        }
        resolve()
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
      case 'session.queued':
      case 'session.queue_update':
        this.options.onState('queued')
        break
      case 'session.queue_done':
        void this.initialize()
        break
      case 'session.created':
        this.ready = true
        this.options.onState('listening')
        void this.options.onReady().catch((error: unknown) => {
          const message =
            error instanceof Error ? error.message : '无法启动麦克风或摄像头'
          this.options.onError(message)
        })
        break
      case 'response.output.delta':
        this.handleDelta(event)
        break
      case 'session.paused':
        this.options.onState('paused')
        break
      case 'session.resumed':
        this.options.onState('listening')
        break
      case 'session.error':
      case 'error':
        this.options.onError(
          event.reason ??
            event.error?.message ??
            event.error?.code ??
            '实时服务返回错误',
        )
        break
      case 'session.closed':
        this.closed = true
        this.options.onState('ended')
        break
    }
  }

  private async initialize() {
    if (this.initialized || !this.transport) return
    this.initialized = true
    this.options.onState('preparing')
    await this.transport.send(
      JSON.stringify({
        type: 'session.init',
        payload: {
          system_prompt:
            this.options.mode === 'video'
              ? '你是一个实时视频语音助手。持续观察画面并听取用户说话，用自然简洁的中文口语回答。'
              : '你是一个实时语音助手。认真听取用户说话，用自然简洁的中文口语回答。',
          config: {
            length_penalty: 1.1,
          },
        },
      }),
    )
  }

  private handleDelta(event: RealtimeEvent) {
    if (event.kind === 'listen') {
      this.userText = event.text?.trim() ?? ''
      if (this.userText) this.options.onUserText(this.userText)
      this.assistantText = ''
      this.options.onState('listening')
      return
    }

    if (event.kind === 'text' && event.text) {
      this.assistantText += event.text
      this.options.onAssistantText(this.assistantText)
      this.options.onState('speaking')
      return
    }

    if (event.kind === 'audio' && event.audio) {
      this.options.onAudio(base64ToFloat32(event.audio))
      this.options.onState('speaking')
    }
  }

  async sendInput(audio: Float32Array, frame: string | null) {
    if (!this.transport || !this.ready || this.closed) return

    const input: Record<string, unknown> = {
      audio: float32ToBase64(audio),
      force_listen: this.forceListenNext,
    }
    this.forceListenNext = false

    if (this.options.mode === 'video' && frame) {
      input.video_frames = [frame]
      input.max_slice_nums = 1
    }

    try {
      await this.transport.send(
        JSON.stringify({
          type: 'input.append',
          input,
        }),
      )
    } catch (error) {
      this.ready = false
      this.closed = true
      const message =
        error instanceof Error ? error.message : '实时音视频发送失败'
      this.options.onError(message)
      try {
        await this.transport.close()
      } catch {
        // The socket may already be gone; the session is closed either way.
      }
    }
  }

  forceListen() {
    this.forceListenNext = true
    this.options.onState('listening')
  }

  async close() {
    if (this.closed) return
    this.closed = true
    if (this.transport) {
      try {
        await this.transport.send(
          JSON.stringify({
            type: 'session.close',
            reason: 'user_stop',
          }),
        )
      } catch {
        // A disconnected socket still counts as a successfully closed session.
      }
      try {
        await this.transport.close()
      } catch {
        // Native disconnect can fail when the peer has already closed.
      }
    }
    this.options.onState('ended')
  }
}
