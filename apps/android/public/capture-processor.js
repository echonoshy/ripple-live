class SecondChunkProcessor extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buffers = []
    this.length = 0
  }

  process(inputs) {
    const input = inputs[0]?.[0]
    if (!input) return true

    const copy = new Float32Array(input)
    this.buffers.push(copy)
    this.length += copy.length

    if (this.length >= sampleRate) {
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
