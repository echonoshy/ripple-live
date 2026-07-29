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

function resampleTo16k(samples: Float32Array, sourceRate: number) {
  const targetRate = 16000
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

export class LiveMedia {
  private readonly options: LiveMediaOptions
  private audioStream: MediaStream | null = null
  private videoStream: MediaStream | null = null
  private captureContext: AudioContext | null = null
  private playbackContext: AudioContext | null = null
  private captureNode: AudioWorkletNode | null = null
  private sourceNode: MediaStreamAudioSourceNode | null = null
  private silentGain: GainNode | null = null
  private muted = false
  private running = false
  private nextPlaybackTime = 0
  private facingMode: 'user' | 'environment'

  constructor(options: LiveMediaOptions) {
    this.options = options
    this.facingMode = options.facingMode
  }

  async start(
    onChunk: (audio: Float32Array, frame: string | null) => void,
  ) {
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error('当前设备不支持麦克风或摄像头采集')
    }

    if (this.options.withVideo) await this.openCamera()

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
      event: MessageEvent<AudioChunkMessage>,
    ) => {
      if (!this.running || event.data.type !== 'audio-chunk') return
      const resampled = resampleTo16k(
        event.data.samples,
        event.data.sampleRate,
      )
      const audio = this.muted
        ? new Float32Array(resampled.length)
        : resampled
      onChunk(audio, this.captureFrame())
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
    if (!samples.length) return
    if (!this.playbackContext) {
      this.playbackContext = new AudioContext()
      this.nextPlaybackTime = this.playbackContext.currentTime + 0.2
    }

    const context = this.playbackContext
    const buffer = context.createBuffer(1, samples.length, 24000)
    buffer.copyToChannel(new Float32Array(samples), 0)
    const source = context.createBufferSource()
    source.buffer = buffer
    source.connect(context.destination)
    const startAt = Math.max(this.nextPlaybackTime, context.currentTime + 0.03)
    source.start(startAt)
    this.nextPlaybackTime = startAt + buffer.duration
  }

  stop() {
    this.running = false
    this.captureNode?.disconnect()
    this.sourceNode?.disconnect()
    this.silentGain?.disconnect()
    this.captureNode = null
    this.sourceNode = null
    this.silentGain = null
    this.audioStream?.getTracks().forEach((track) => track.stop())
    this.audioStream = null
    void this.captureContext?.close()
    this.captureContext = null
    void this.playbackContext?.close()
    this.playbackContext = null
    this.nextPlaybackTime = 0
    this.stopCamera()
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
