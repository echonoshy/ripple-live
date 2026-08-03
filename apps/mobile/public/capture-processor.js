class SecondChunkProcessor extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buffers = []
    this.length = 0
    this.levelSquareSum = 0
    this.levelSampleCount = 0
  }

  process(inputs) {
    const input = inputs[0]?.[0]
    if (!input) return true

    const copy = new Float32Array(input)
    this.buffers.push(copy)
    this.length += copy.length

    for (const sample of input) {
      this.levelSquareSum += sample * sample
    }
    this.levelSampleCount += input.length

    if (this.levelSampleCount >= sampleRate / 20) {
      this.port.postMessage({
        type: 'audio-level',
        level: Math.sqrt(this.levelSquareSum / this.levelSampleCount),
      })
      this.levelSquareSum = 0
      this.levelSampleCount = 0
    }

    if (this.length >= sampleRate / 10) {
      const chunk = new Float32Array(this.length)
      let offset = 0
      for (const buffer of this.buffers) {
        chunk.set(buffer, offset)
        offset += buffer.length
      }
      this.buffers = []
      this.length = 0
      this.port.postMessage(
        {
          type: 'audio-chunk',
          samples: chunk,
          sampleRate,
        },
        [chunk.buffer],
      )
    }

    return true
  }
}

registerProcessor('second-chunk-processor', SecondChunkProcessor)
