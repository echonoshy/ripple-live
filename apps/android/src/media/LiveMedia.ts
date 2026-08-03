import { MicVAD } from '@ricky0123/vad-web'

type LiveMediaOptions = {
  video: HTMLVideoElement
  canvas: HTMLCanvasElement
  withVideo: boolean
  facingMode: 'user' | 'environment'
}

type AudioChunkMessage = {
  type: 'audio-chunk'
  samples: Float32Array
  sampleRate: number
}

type AudioLevelMessage = {
  type: 'audio-level'
  level: number
}

type PlaybackStateMessage = {
  type: 'playback-started' | 'playback-underrun'
  bufferedMs?: number
  count?: number
}

function resample(
  samples: Float32Array,
  sourceRate: number,
  targetRate: number,
) {
  if (sourceRate === targetRate) return samples

  const outputLength = Math.max(
    1,
    Math.round((samples.length * targetRate) / sourceRate),
  )
  const output = new Float32Array(outputLength)
  const ratio = sourceRate / targetRate

  for (let index = 0; index < outputLength; index += 1) {
    const position = index * ratio
    const left = Math.floor(position)
    const right = Math.min(left + 1, samples.length - 1)
    const mix = position - left
    output[index] = samples[left] * (1 - mix) + samples[right] * mix
  }
  return output
}

function resampleTo16k(samples: Float32Array, sourceRate: number) {
  return resample(samples, sourceRate, 16000)
}

export class LiveMedia {
  private readonly options: LiveMediaOptions
  private audioStream: MediaStream | null = null
  private videoStream: MediaStream | null = null
  private captureContext: AudioContext | null = null
  private playbackContext: AudioContext | null = null
  private captureNode: AudioWorkletNode | null = null
  private playbackNode: AudioWorkletNode | null = null
  private sourceNode: MediaStreamAudioSourceNode | null = null
  private silentGain: GainNode | null = null
  private vad: MicVAD | null = null
  private muted = false
  private running = false
  private facingMode: 'user' | 'environment'
  private lastFrameAt = 0
  private fallbackVad = false
  private fallbackSpeaking = false
  private fallbackSilenceChunks = 0

  constructor(options: LiveMediaOptions) {
    this.options = options
    this.facingMode = options.facingMode
  }

  async start(
    onChunk: (audio: Float32Array, frame: string | null) => void,
    onSpeechStart: () => void,
    onSpeechEnd: () => void,
    onLevel: (level: number) => void,
  ) {
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error('当前设备不支持麦克风或摄像头采集')
    }

    if (this.options.withVideo) await this.openCamera()

    await this.openPlayback()

    this.audioStream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
      video: false,
    })

    this.captureContext = new AudioContext()
    await this.captureContext.audioWorklet.addModule('/capture-processor.js')
    if (this.captureContext.state === 'suspended') {
      await this.captureContext.resume()
    }

    this.sourceNode = this.captureContext.createMediaStreamSource(this.audioStream)
    this.captureNode = new AudioWorkletNode(
      this.captureContext,
      'second-chunk-processor',
    )
    this.silentGain = this.captureContext.createGain()
    this.silentGain.gain.value = 0
    this.sourceNode.connect(this.captureNode)
    this.captureNode.connect(this.silentGain)
    this.silentGain.connect(this.captureContext.destination)
    this.running = true

    this.captureNode.port.onmessage = (
      event: MessageEvent<AudioChunkMessage | AudioLevelMessage>,
    ) => {
      if (!this.running) return
      if (event.data.type === 'audio-level') {
        onLevel(this.muted ? 0 : Math.min(1, event.data.level * 8))
        return
      }
      const resampled = resampleTo16k(
        event.data.samples,
        event.data.sampleRate,
      )
      const audio = this.muted
        ? new Float32Array(resampled.length)
        : resampled
      if (this.fallbackVad && !this.muted) {
        let squareSum = 0
        for (const sample of audio) squareSum += sample * sample
        const rms = Math.sqrt(squareSum / Math.max(1, audio.length))
        if (!this.fallbackSpeaking && rms >= 0.02) {
          this.fallbackSpeaking = true
          this.fallbackSilenceChunks = 0
          onSpeechStart()
        } else if (this.fallbackSpeaking) {
          this.fallbackSilenceChunks =
            rms < 0.012 ? this.fallbackSilenceChunks + 1 : 0
          if (this.fallbackSilenceChunks >= 6) {
            this.fallbackSpeaking = false
            this.fallbackSilenceChunks = 0
            onSpeechEnd()
          }
        }
      }
      onChunk(audio, this.captureFrame())
    }

    try {
      this.vad = await MicVAD.new({
        model: 'v5',
        baseAssetPath: '/vad/',
        onnxWASMBasePath: '/vad/ort/',
        getStream: async () => {
          if (!this.audioStream) throw new Error('麦克风流尚未启动')
          return this.audioStream
        },
        pauseStream: async () => {},
        resumeStream: async (stream) => stream,
        positiveSpeechThreshold: 0.6,
        negativeSpeechThreshold: 0.35,
        redemptionMs: 320,
        minSpeechMs: 96,
        preSpeechPadMs: 0,
        ortConfig: (ort) => {
          ort.env.wasm.numThreads = 1
        },
        onSpeechRealStart: () => {
          if (this.running && !this.muted) onSpeechStart()
        },
        onSpeechEnd: () => {
          if (this.running && !this.muted) onSpeechEnd()
        },
      })
    } catch (error) {
      console.warn('Silero VAD 初始化失败，已切换到本地能量检测', error)
      this.vad = null
      this.fallbackVad = true
    }
  }

  setMuted(muted: boolean) {
    this.muted = muted
  }

  async setFacingMode(facingMode: 'user' | 'environment') {
    this.facingMode = facingMode
    if (!this.options.withVideo) return
    this.stopCamera()
    await this.openCamera()
  }

  enqueueOutput(samples: Float32Array) {
    if (!samples.length || !this.playbackNode || !this.playbackContext) return
    const output =
      this.playbackContext.sampleRate === 24000
        ? new Float32Array(samples)
        : resample(samples, 24000, this.playbackContext.sampleRate)
    this.playbackNode.port.postMessage(
      { type: 'enqueue', samples: output },
      [output.buffer],
    )
  }

  finishOutput() {
    this.playbackNode?.port.postMessage({ type: 'end' })
  }

  clearOutput() {
    this.playbackNode?.port.postMessage({ type: 'clear' })
  }

  stop() {
    this.running = false
    this.fallbackVad = false
    this.fallbackSpeaking = false
    this.fallbackSilenceChunks = 0
    const vad = this.vad
    this.vad = null
    void vad?.destroy().catch(() => {})
    this.clearOutput()
    this.captureNode?.disconnect()
    this.playbackNode?.disconnect()
    this.sourceNode?.disconnect()
    this.silentGain?.disconnect()
    this.captureNode = null
    this.playbackNode = null
    this.sourceNode = null
    this.silentGain = null
    this.audioStream?.getTracks().forEach((track) => track.stop())
    this.audioStream = null
    void this.captureContext?.close()
    this.captureContext = null
    void this.playbackContext?.close()
    this.playbackContext = null
    this.stopCamera()
  }

  private async openPlayback() {
    this.playbackContext = new AudioContext({
      latencyHint: 'interactive',
      sampleRate: 24000,
    })
    await this.playbackContext.audioWorklet.addModule('/playback-processor.js')
    this.playbackNode = new AudioWorkletNode(
      this.playbackContext,
      'stream-playback-processor',
      {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        processorOptions: {
          initialBufferMs: 450,
          rebufferMs: 300,
        },
      },
    )
    this.playbackNode.port.onmessage = (
      event: MessageEvent<PlaybackStateMessage>,
    ) => {
      if (event.data.type === 'playback-underrun') {
        console.warn('[Ripple Live] playback buffer underrun', {
          count: event.data.count,
        })
      } else if (event.data.type === 'playback-started') {
        console.info('[Ripple Live] buffered playback started', {
          bufferedMs: event.data.bufferedMs,
          sampleRate: this.playbackContext?.sampleRate,
        })
      }
    }
    this.playbackNode.connect(this.playbackContext.destination)
    if (this.playbackContext.state === 'suspended') {
      await this.playbackContext.resume()
    }
  }

  private async openCamera() {
    this.videoStream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: {
        facingMode: { ideal: this.facingMode },
        width: { ideal: 1280 },
        height: { ideal: 720 },
      },
    })
    this.options.video.srcObject = this.videoStream
    this.options.video.style.transform =
      this.facingMode === 'user' ? 'scaleX(-1)' : 'none'
    await this.options.video.play()
  }

  private stopCamera() {
    this.videoStream?.getTracks().forEach((track) => track.stop())
    this.videoStream = null
    this.options.video.pause()
    this.options.video.srcObject = null
  }

  private captureFrame() {
    const now = performance.now()
    if (now - this.lastFrameAt < 1000) return null
    if (
      !this.options.withVideo ||
      !this.options.video.videoWidth ||
      !this.options.video.videoHeight
    ) {
      return null
    }

    const sourceWidth = this.options.video.videoWidth
    const sourceHeight = this.options.video.videoHeight
    const width = Math.min(640, sourceWidth)
    const height = Math.round((sourceHeight / sourceWidth) * width)
    this.options.canvas.width = width
    this.options.canvas.height = height
    const context = this.options.canvas.getContext('2d')
    if (!context) return null
    context.drawImage(this.options.video, 0, 0, width, height)
    this.lastFrameAt = now
    return this.options.canvas.toDataURL('image/jpeg', 0.7).split(',')[1] ?? null
  }
}
