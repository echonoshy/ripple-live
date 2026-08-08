export type CameraFacingMode = 'user' | 'environment'
export type CameraEnableResult = 'enabled' | 'stale'

type TimerDependencies = {
  setTimeout(callback: () => void, timeoutMs: number): unknown
  clearTimeout(handle: unknown): void
}

export type CameraDependencies = {
  getUserMedia(constraints: MediaStreamConstraints): Promise<MediaStream>
  waitForFirstFrame(
    video: HTMLVideoElement,
    timeoutMs: number,
    signal?: AbortSignal,
  ): Promise<void>
  onInterrupted?: () => void
  videoConstraints?: Pick<MediaTrackConstraints, 'width' | 'height'>
}

const browserTimers: TimerDependencies = {
  setTimeout: (callback, timeoutMs) => globalThis.setTimeout(callback, timeoutMs),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as number),
}

export function waitForFirstFrame(
  video: HTMLVideoElement,
  timeoutMs: number,
  timers: TimerDependencies = browserTimers,
  signal?: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false
    let timeoutHandle: unknown

    const cleanup = () => {
      video.removeEventListener('loadeddata', onLoadedData)
      video.removeEventListener('error', onError)
      signal?.removeEventListener('abort', onAbort)
      if (timeoutHandle !== undefined) timers.clearTimeout(timeoutHandle)
    }
    const settle = (error?: Error) => {
      if (settled) return
      settled = true
      cleanup()
      if (error) reject(error)
      else resolve()
    }
    const onLoadedData = () => settle()
    const onError = () => settle(new Error('摄像头画面加载失败'))
    const onAbort = () => settle(new DOMException('Aborted', 'AbortError'))

    video.addEventListener('loadeddata', onLoadedData)
    video.addEventListener('error', onError)
    signal?.addEventListener('abort', onAbort, { once: true })
    if (signal?.aborted) {
      onAbort()
      return
    }
    if (video.readyState >= 2 && video.videoWidth > 0) {
      try {
        void Promise.resolve(video.play()).catch(() => onError())
      } catch {
        onError()
        return
      }
      settle()
      return
    }
    timeoutHandle = timers.setTimeout(
      () => settle(new Error('摄像头画面加载超时')),
      timeoutMs,
    )
    try {
      void Promise.resolve(video.play()).catch(() => onError())
    } catch {
      onError()
    }
  })
}

function videoTracks(stream: MediaStream) {
  if (typeof stream.getVideoTracks === 'function') return stream.getVideoTracks()
  return stream.getTracks().filter((track) => track.kind === 'video')
}

export class CameraController {
  private readonly video: HTMLVideoElement
  private readonly deps: CameraDependencies
  private stream: MediaStream | null = null
  private facingMode: CameraFacingMode | null = null
  private operation = 0
  private pending:
    | {
        facingMode: CameraFacingMode
        promise: Promise<CameraEnableResult>
      }
    | null = null
  private readonly pendingStreams = new Set<MediaStream>()
  private readonly disposedStreams = new WeakSet<MediaStream>()
  private removeTrackListeners: (() => void) | null = null
  private firstFrameAbort: AbortController | null = null

  constructor(
    video: HTMLVideoElement,
    deps: CameraDependencies,
  ) {
    this.video = video
    this.deps = deps
  }

  get enabled() {
    return this.stream !== null
  }

  async enable(facingMode: CameraFacingMode): Promise<CameraEnableResult> {
    if (this.pending?.facingMode === facingMode) return this.pending.promise
    if (this.stream && this.facingMode === facingMode) {
      if (this.pending) {
        this.operation += 1
        this.pending = null
        this.abortFirstFrameWait()
        for (const stream of this.pendingStreams) this.dispose(stream)
        this.pendingStreams.clear()
        this.video.srcObject = this.stream
        this.video.style.transform =
          facingMode === 'user' ? 'scaleX(-1)' : 'none'
        void this.video.play().catch(() => {})
      }
      return 'enabled'
    }

    this.abortFirstFrameWait()
    const operation = ++this.operation
    const promise = this.enableInternal(facingMode, operation)
    this.pending = { facingMode, promise }
    void promise.finally(() => {
      if (this.pending?.promise === promise) this.pending = null
    }).catch(() => {})
    return promise
  }

  disable() {
    this.operation += 1
    this.pending = null
    this.abortFirstFrameWait()
    for (const stream of this.pendingStreams) this.dispose(stream)
    this.pendingStreams.clear()
    this.disposeCurrent()
    this.video.pause()
    this.video.srcObject = null
  }

  private async enableInternal(
    facingMode: CameraFacingMode,
    operation: number,
  ): Promise<CameraEnableResult> {
    let replacement: MediaStream
    try {
      replacement = await this.deps.getUserMedia({
        audio: false,
        video: {
          facingMode: { ideal: facingMode },
          width: { ideal: 1280 },
          height: { ideal: 720 },
          ...this.deps.videoConstraints,
        },
      })
    } catch (error) {
      if (!this.isCurrent(operation)) return 'stale'
      throw error
    }

    this.pendingStreams.add(replacement)
    if (!this.isCurrent(operation)) {
      this.disposePending(replacement)
      return 'stale'
    }

    const previous = this.stream
    const previousTransform = this.video.style.transform
    this.video.srcObject = replacement
    this.video.style.transform = facingMode === 'user' ? 'scaleX(-1)' : 'none'
    const firstFrameAbort = new AbortController()
    this.firstFrameAbort = firstFrameAbort
    try {
      await this.deps.waitForFirstFrame(
        this.video,
        3000,
        firstFrameAbort.signal,
      )
    } catch (error) {
      this.restore(previous, previousTransform, replacement)
      this.disposePending(replacement)
      if (!this.isCurrent(operation)) return 'stale'
      throw error
    } finally {
      if (this.firstFrameAbort === firstFrameAbort) {
        this.firstFrameAbort = null
      }
    }

    if (!this.isCurrent(operation)) {
      this.restore(previous, previousTransform, replacement)
      this.disposePending(replacement)
      return 'stale'
    }

    if (videoTracks(replacement).some((track) => track.readyState === 'ended')) {
      this.restore(previous, previousTransform, replacement)
      this.disposePending(replacement)
      throw new Error('摄像头已中断')
    }

    this.pendingStreams.delete(replacement)
    this.removeTrackListeners?.()
    this.stream = replacement
    this.facingMode = facingMode
    this.bindTrackInterruptions(replacement)
    if (previous && previous !== replacement) this.dispose(previous)
    return 'enabled'
  }

  private bindTrackInterruptions(stream: MediaStream) {
    const tracks = videoTracks(stream)
    const onEnded = () => {
      if (this.stream !== stream) return
      this.operation += 1
      this.disposeCurrent()
      if (this.video.srcObject === stream) {
        this.video.pause()
        this.video.srcObject = null
      }
      this.deps.onInterrupted?.()
    }
    for (const track of tracks) {
      track.addEventListener('ended', onEnded)
      track.addEventListener('mute', onEnded)
    }
    this.removeTrackListeners = () => {
      for (const track of tracks) {
        track.removeEventListener('ended', onEnded)
        track.removeEventListener('mute', onEnded)
      }
    }
  }

  private disposeCurrent() {
    const current = this.stream
    this.removeTrackListeners?.()
    this.removeTrackListeners = null
    this.stream = null
    this.facingMode = null
    if (current) this.dispose(current)
  }

  private disposePending(stream: MediaStream) {
    this.pendingStreams.delete(stream)
    this.dispose(stream)
    if (this.video.srcObject === stream) this.video.srcObject = null
  }

  private dispose(stream: MediaStream) {
    if (this.disposedStreams.has(stream)) return
    this.disposedStreams.add(stream)
    for (const track of videoTracks(stream)) track.stop()
  }

  private restore(
    previous: MediaStream | null,
    previousTransform: string,
    replacement: MediaStream,
  ) {
    if (this.video.srcObject !== replacement) return
    this.video.srcObject = previous
    this.video.style.transform = previousTransform
    if (previous) void this.video.play().catch(() => {})
    else this.video.pause()
  }

  private isCurrent(operation: number) {
    return this.operation === operation
  }

  private abortFirstFrameWait() {
    this.firstFrameAbort?.abort()
    this.firstFrameAbort = null
  }
}
