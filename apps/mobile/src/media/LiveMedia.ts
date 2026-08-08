import { MicVAD } from '@ricky0123/vad-web'

type LiveMediaOptions = {
  video: HTMLVideoElement
  canvas: HTMLCanvasElement
  withVideo: boolean
  facingMode: 'user' | 'environment'
  onPlaybackStarted: (bufferedMs: number) => void
  onPlaybackEnded: () => void
  onOutputLevel: (level: number) => void
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
  type:
    | 'playback-started'
    | 'playback-ended'
    | 'playback-underrun'
    | 'audio-level'
  level?: number
  bufferedMs?: number
  count?: number
}

type CameraSwitchResult = 'switched' | 'stale' | 'failed'

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
  private fallbackVad = false
  private fallbackSpeaking = false
  private fallbackSilenceChunks = 0
  private speechActive = false
  private preRoll: Float32Array[] = []
  private preRollSamples = 0
  private lifecycleGeneration = 0
  private cameraGeneration = 0
  private startPromise: Promise<void> | null = null
  private pendingStreams = new Set<MediaStream>()
  private pendingContexts = new Set<AudioContext>()

  constructor(options: LiveMediaOptions) {
    this.options = options
    this.facingMode = options.facingMode
  }

  start(
    onChunk: (audio: Float32Array, frame: string | null) => void,
    onSpeechStart: () => void,
    onSpeechEnd: () => void,
    onLevel: (level: number) => void,
  ): Promise<void> {
    if (!navigator.mediaDevices?.getUserMedia) {
      return Promise.reject(new Error('当前设备不支持麦克风或摄像头采集'))
    }
    if (this.startPromise) return this.startPromise
    if (this.running) return Promise.resolve()

    const generation = ++this.lifecycleGeneration
    const cameraOperation = ++this.cameraGeneration
    const task = this.startInternal(
      generation,
      cameraOperation,
      onChunk,
      onSpeechStart,
      onSpeechEnd,
      onLevel,
    ).finally(() => {
      if (this.startPromise === task) this.startPromise = null
    })
    this.startPromise = task
    return task
  }

  private async startInternal(
    generation: number,
    cameraOperation: number,
    onChunk: (audio: Float32Array, frame: string | null) => void,
    onSpeechStart: () => void,
    onSpeechEnd: () => void,
    onLevel: (level: number) => void,
  ) {
    try {
      if (this.options.withVideo) {
        await this.replaceCamera(
          this.facingMode,
          generation,
          cameraOperation,
        )
        if (!this.isCurrent(generation)) return
      }

      if (!await this.openPlayback(generation)) return

      const audioStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
        video: false,
      })
      this.pendingStreams.add(audioStream)
      if (!this.isCurrent(generation)) {
        this.disposePendingStream(audioStream)
        return
      }
      this.pendingStreams.delete(audioStream)
      this.audioStream = audioStream

      const captureContext = new AudioContext()
      this.pendingContexts.add(captureContext)
      await captureContext.audioWorklet.addModule('/capture-processor.js')
      if (!this.isCurrent(generation)) {
        this.disposePendingContext(captureContext)
        return
      }
      if (captureContext.state === 'suspended') await captureContext.resume()
      if (!this.isCurrent(generation)) {
        this.disposePendingContext(captureContext)
        return
      }

      const sourceNode = captureContext.createMediaStreamSource(audioStream)
      const captureNode = new AudioWorkletNode(
        captureContext,
        'second-chunk-processor',
      )
      const silentGain = captureContext.createGain()
      silentGain.gain.value = 0
      sourceNode.connect(captureNode)
      captureNode.connect(silentGain)
      silentGain.connect(captureContext.destination)
      this.pendingContexts.delete(captureContext)
      this.captureContext = captureContext
      this.sourceNode = sourceNode
      this.captureNode = captureNode
      this.silentGain = silentGain
      this.running = true

      captureNode.port.onmessage = (
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
        const beginSpeech = () => {
          if (this.speechActive || this.muted) return
          this.speechActive = true
          onSpeechStart()
          for (const buffered of this.preRoll) onChunk(buffered, null)
          this.preRoll = []
          this.preRollSamples = 0
        }
        const endSpeech = () => {
          if (!this.speechActive) return
          this.speechActive = false
          onSpeechEnd()
        }
        if (this.fallbackVad && !this.muted) {
          let squareSum = 0
          for (const sample of audio) squareSum += sample * sample
          const rms = Math.sqrt(squareSum / Math.max(1, audio.length))
          if (!this.fallbackSpeaking && rms >= 0.02) {
            this.fallbackSpeaking = true
            this.fallbackSilenceChunks = 0
            beginSpeech()
          } else if (this.fallbackSpeaking) {
            this.fallbackSilenceChunks =
              rms < 0.012 ? this.fallbackSilenceChunks + 1 : 0
            if (this.fallbackSilenceChunks >= 8) {
              this.fallbackSpeaking = false
              this.fallbackSilenceChunks = 0
              endSpeech()
            }
          }
        }
        if (this.speechActive) {
          onChunk(audio, null)
        } else if (!this.muted) {
          this.preRoll.push(audio)
          this.preRollSamples += audio.length
          const maxPreRollSamples = 16_000
          while (
            this.preRollSamples > maxPreRollSamples &&
            this.preRoll.length > 1
          ) {
            this.preRollSamples -= this.preRoll.shift()?.length ?? 0
          }
        }
      }

      let vad: MicVAD
      try {
        vad = await MicVAD.new({
          model: 'v5',
          baseAssetPath: '/vad/',
          onnxWASMBasePath: '/vad/ort/',
          getStream: async () => {
            if (!this.isCurrent(generation)) {
              throw new Error('麦克风采集已停止')
            }
            return audioStream
          },
          pauseStream: async () => {},
          resumeStream: async (stream) => stream,
          positiveSpeechThreshold: 0.6,
          negativeSpeechThreshold: 0.35,
          redemptionMs: 500,
          minSpeechMs: 96,
          preSpeechPadMs: 0,
          ortConfig: (ort) => {
            ort.env.wasm.numThreads = 1
          },
          onSpeechRealStart: () => {
            if (this.running && !this.muted && !this.speechActive) {
              this.speechActive = true
              onSpeechStart()
              for (const buffered of this.preRoll) onChunk(buffered, null)
              this.preRoll = []
              this.preRollSamples = 0
            }
          },
          onSpeechEnd: () => {
            if (this.running && !this.muted && this.speechActive) {
              this.speechActive = false
              onSpeechEnd()
            }
          },
        })
      } catch (error) {
        if (!this.isCurrent(generation)) return
        console.warn('Silero VAD 初始化失败，已切换到本地能量检测', error)
        this.vad = null
        this.fallbackVad = true
        return
      }
      if (!this.isCurrent(generation)) {
        void vad.destroy().catch(() => {})
        return
      }
      this.vad = vad
    } catch (error) {
      if (!this.isCurrent(generation)) return
      this.stop()
      throw error
    }
  }

  setMuted(muted: boolean) {
    this.muted = muted
    if (muted) {
      this.speechActive = false
      this.preRoll = []
      this.preRollSamples = 0
    }
  }

  async setFacingMode(
    facingMode: 'user' | 'environment',
  ): Promise<CameraSwitchResult> {
    if (!this.options.withVideo) {
      this.facingMode = facingMode
      return 'switched'
    }
    const generation = this.lifecycleGeneration
    const cameraOperation = ++this.cameraGeneration
    try {
      return await this.replaceCamera(
        facingMode,
        generation,
        cameraOperation,
      )
    } catch {
      return this.isCameraCurrent(generation, cameraOperation)
        ? 'failed'
        : 'stale'
    }
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
    this.options.onOutputLevel(0)
  }

  stop() {
    this.lifecycleGeneration += 1
    this.cameraGeneration += 1
    this.startPromise = null
    this.running = false
    this.fallbackVad = false
    this.fallbackSpeaking = false
    this.fallbackSilenceChunks = 0
    this.speechActive = false
    this.preRoll = []
    this.preRollSamples = 0
    const vad = this.vad
    this.vad = null
    void vad?.destroy().catch(() => {})
    this.clearOutput()
    this.options.onOutputLevel(0)
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
    for (const stream of this.pendingStreams) {
      stream.getTracks().forEach((track) => track.stop())
    }
    this.pendingStreams.clear()
    void this.captureContext?.close()
    this.captureContext = null
    void this.playbackContext?.close()
    this.playbackContext = null
    for (const context of this.pendingContexts) void context.close().catch(() => {})
    this.pendingContexts.clear()
    this.stopCamera()
  }

  private async openPlayback(generation = this.lifecycleGeneration) {
    const playbackContext = new AudioContext({
      latencyHint: 'interactive',
      sampleRate: 24000,
    })
    this.pendingContexts.add(playbackContext)
    await playbackContext.audioWorklet.addModule('/playback-processor.js')
    if (!this.isCurrent(generation)) {
      this.disposePendingContext(playbackContext)
      return false
    }
    const playbackNode = new AudioWorkletNode(
      playbackContext,
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
    playbackNode.port.onmessage = (
      event: MessageEvent<PlaybackStateMessage>,
    ) => {
      if (event.data.type === 'audio-level') {
        event.data.level = Number.isFinite(event.data.level)
          ? Math.min(1, Math.max(0, event.data.level ?? 0))
          : 0
        this.options.onOutputLevel(event.data.level)
        return
      }
      if (event.data.type === 'playback-underrun') {
        console.warn('[Ripple Live] playback buffer underrun', {
          count: event.data.count,
        })
      } else if (event.data.type === 'playback-started') {
        this.options.onPlaybackStarted(event.data.bufferedMs ?? 0)
        console.info('[Ripple Live] buffered playback started', {
          bufferedMs: event.data.bufferedMs,
          sampleRate: playbackContext.sampleRate,
        })
      } else if (event.data.type === 'playback-ended') {
        this.options.onPlaybackEnded()
      }
    }
    playbackNode.connect(playbackContext.destination)
    if (playbackContext.state === 'suspended') {
      await playbackContext.resume()
    }
    if (!this.isCurrent(generation)) {
      playbackNode.disconnect()
      this.disposePendingContext(playbackContext)
      return false
    }
    this.pendingContexts.delete(playbackContext)
    this.playbackContext = playbackContext
    this.playbackNode = playbackNode
    return true
  }

  private async replaceCamera(
    facingMode: 'user' | 'environment',
    generation: number,
    cameraOperation: number,
  ) {
    const replacement = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: {
        facingMode: { ideal: facingMode },
        width: { ideal: 1280 },
        height: { ideal: 720 },
      },
    })
    this.pendingStreams.add(replacement)
    if (!this.isCameraCurrent(generation, cameraOperation)) {
      this.disposePendingStream(replacement)
      return 'stale'
    }

    const previous = this.videoStream
    const previousTransform = this.options.video.style.transform
    this.options.video.srcObject = replacement
    this.options.video.style.transform =
      facingMode === 'user' ? 'scaleX(-1)' : 'none'
    try {
      await this.options.video.play()
    } catch (error) {
      this.disposePendingStream(replacement)
      if (this.isCurrent(generation)) {
        this.restoreCamera(previous, previousTransform, replacement)
      }
      if (this.isCameraCurrent(generation, cameraOperation)) {
        throw error
      }
      return 'stale'
    }

    if (!this.isCameraCurrent(generation, cameraOperation)) {
      this.disposePendingStream(replacement)
      if (this.isCurrent(generation)) {
        this.restoreCamera(previous, previousTransform, replacement)
      }
      return 'stale'
    }

    this.pendingStreams.delete(replacement)
    this.videoStream = replacement
    this.facingMode = facingMode
    if (previous && previous !== replacement) {
      previous.getTracks().forEach((track) => track.stop())
    }
    return 'switched'
  }

  private stopCamera() {
    this.videoStream?.getTracks().forEach((track) => track.stop())
    this.videoStream = null
    this.options.video.pause()
    this.options.video.srcObject = null
  }

  private restoreCamera(
    previous: MediaStream | null,
    previousTransform: string,
    replacement: MediaStream,
  ) {
    if (this.options.video.srcObject !== replacement) return
    this.options.video.srcObject = previous
    this.options.video.style.transform = previousTransform
    if (previous) void this.options.video.play().catch(() => {})
    else this.options.video.pause()
  }

  private isCurrent(generation: number) {
    return this.lifecycleGeneration === generation
  }

  private isCameraCurrent(generation: number, cameraOperation: number) {
    return this.isCurrent(generation) && this.cameraGeneration === cameraOperation
  }

  private disposePendingStream(stream: MediaStream) {
    if (!this.pendingStreams.delete(stream)) return
    stream.getTracks().forEach((track) => track.stop())
  }

  private disposePendingContext(context: AudioContext) {
    if (!this.pendingContexts.delete(context)) return
    void context.close().catch(() => {})
  }

  captureFrame() {
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
    return this.options.canvas.toDataURL('image/jpeg', 0.7).split(',')[1] ?? null
  }
}
