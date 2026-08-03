class StreamPlaybackProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super()
    const config = options.processorOptions ?? {}
    this.initialBufferSamples = Math.round(
      sampleRate * ((config.initialBufferMs ?? 450) / 1000),
    )
    this.rebufferSamples = Math.round(
      sampleRate * ((config.rebufferMs ?? 300) / 1000),
    )
    this.chunks = []
    this.chunkOffset = 0
    this.queuedSamples = 0
    this.started = false
    this.playing = false
    this.ending = false
    this.underruns = 0

    this.port.onmessage = (event) => {
      if (event.data?.type === 'enqueue') {
        const samples = event.data.samples
        if (samples?.length) {
          this.chunks.push(samples)
          this.queuedSamples += samples.length
          this.ending = false
        }
      } else if (event.data?.type === 'end') {
        this.ending = true
      } else if (event.data?.type === 'clear') {
        this.chunks = []
        this.chunkOffset = 0
        this.queuedSamples = 0
        this.started = false
        this.playing = false
        this.ending = false
      }
    }
  }

  process(_inputs, outputs) {
    const output = outputs[0]?.[0]
    if (!output) return true
    output.fill(0)

    if (!this.playing) {
      const threshold = this.started
        ? this.rebufferSamples
        : this.initialBufferSamples
      if (this.queuedSamples < threshold && !this.ending) return true
      if (!this.queuedSamples) return true
      this.playing = true
      this.started = true
      this.port.postMessage({
        type: 'playback-started',
        bufferedMs: Math.round((this.queuedSamples * 1000) / sampleRate),
      })
    }

    let outputOffset = 0
    while (outputOffset < output.length && this.chunks.length) {
      const chunk = this.chunks[0]
      const available = chunk.length - this.chunkOffset
      const count = Math.min(available, output.length - outputOffset)
      output.set(
        chunk.subarray(this.chunkOffset, this.chunkOffset + count),
        outputOffset,
      )
      outputOffset += count
      this.chunkOffset += count
      this.queuedSamples -= count
      if (this.chunkOffset === chunk.length) {
        this.chunks.shift()
        this.chunkOffset = 0
      }
    }

    if (outputOffset < output.length) {
      this.playing = false
      if (!this.ending) {
        this.underruns += 1
        this.port.postMessage({
          type: 'playback-underrun',
          count: this.underruns,
        })
      }
    }
    return true
  }
}

registerProcessor('stream-playback-processor', StreamPlaybackProcessor)
