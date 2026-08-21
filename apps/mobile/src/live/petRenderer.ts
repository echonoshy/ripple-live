import type { VisualState } from './motion'
import type { OrbFrame, OrbRenderer } from './orbRenderer'

export const PET_ATLAS = {
  width: 1536,
  height: 2288,
  columns: 8,
  rows: 11,
  cellWidth: 192,
  cellHeight: 208,
} as const

export const PET_HD_ATLAS = {
  width: 3072,
  height: 2496,
  columns: 8,
  rows: 6,
  cellWidth: 384,
  cellHeight: 416,
} as const

export const PET_HD_ROWS: Record<VisualState, number> = {
  idle: 0,
  connecting: 3,
  listening: 3,
  thinking: 4,
  tool: 5,
  speaking: 0,
  ended: 1,
  error: 2,
}

export type PetGifState =
  | 'idle'
  | 'waving'
  | 'failed'
  | 'waiting'
  | 'running'
  | 'review'
export type PetGifUrls = Record<PetGifState, string>

export const PET_GIF_STATES: Record<VisualState, PetGifState> = {
  idle: 'idle',
  connecting: 'waiting',
  listening: 'waiting',
  thinking: 'running',
  tool: 'review',
  speaking: 'idle',
  ended: 'waving',
  error: 'failed',
}

type PetAnimation = {
  row: number
  durations: readonly number[]
  loop: boolean
}

const idle = [280, 110, 110, 140, 140, 320] as const
const waving = [140, 140, 140, 280] as const
const failed = [140, 140, 140, 140, 140, 140, 140, 240] as const
const waiting = [150, 150, 150, 150, 150, 260] as const
const running = [120, 120, 120, 120, 120, 220] as const
const review = [150, 150, 150, 150, 150, 280] as const

export const PET_ANIMATIONS: Record<VisualState, PetAnimation> = {
  idle: { row: 0, durations: idle, loop: true },
  connecting: { row: 6, durations: waiting, loop: true },
  listening: { row: 6, durations: waiting, loop: true },
  thinking: { row: 7, durations: running, loop: true },
  tool: { row: 8, durations: review, loop: true },
  speaking: { row: 0, durations: idle, loop: true },
  ended: { row: 3, durations: waving, loop: false },
  error: { row: 5, durations: failed, loop: false },
}

const clamp = (value: number, minimum: number, maximum: number) => {
  if (!Number.isFinite(value)) return minimum
  return Math.min(maximum, Math.max(minimum, value))
}

export function petFrameAt(
  state: VisualState,
  elapsedMs: number,
  reducedMotion: boolean,
) {
  const animation = PET_ANIMATIONS[state]
  if (reducedMotion) return { row: animation.row, column: 0 }

  const totalMs = animation.durations.reduce((sum, duration) => sum + duration, 0)
  const safeElapsed = clamp(elapsedMs, 0, Number.MAX_SAFE_INTEGER)
  const positionMs = animation.loop
    ? safeElapsed % totalMs
    : Math.min(safeElapsed, Math.max(0, totalMs - 1))

  let boundaryMs = 0
  for (let column = 0; column < animation.durations.length; column += 1) {
    boundaryMs += animation.durations[column]
    if (positionMs < boundaryMs) return { row: animation.row, column }
  }
  return { row: animation.row, column: animation.durations.length - 1 }
}

function reactiveLevel(frame: OrbFrame) {
  if (frame.state === 'listening') return clamp(frame.inputLevel, 0, 1)
  if (frame.state === 'speaking') return clamp(frame.outputLevel, 0, 1)
  return 0
}

export function createPetRenderer(
  canvas: HTMLCanvasElement,
  spriteSheetUrl: string,
  hdSpriteSheetUrl?: string,
  gifUrls?: PetGifUrls,
): OrbRenderer {
  const context = canvas.getContext('2d', { alpha: true })
  if (!context) throw new Error('canvas_2d_unavailable')

  const spriteSheet = new Image()
  const hdSpriteSheet = hdSpriteSheetUrl ? new Image() : null
  let disposed = false
  let loaded = false
  let hdLoaded = false
  let loadFailed = false
  let stateStartedAt = 0
  let previousState: VisualState | null = null
  let previousNowMs = 0
  let lastFrame: OrbFrame | null = null
  let activeGif: HTMLImageElement | null = null
  let activeGifState: PetGifState | null = null
  let activeGifLoaded = false

  const activateGif = (state: PetGifState) => {
    if (!gifUrls || activeGifState === state) return
    activeGifState = state
    activeGifLoaded = false
    const image = new Image()
    activeGif = image
    image.onload = () => {
      if (disposed || activeGif !== image) return
      activeGifLoaded = true
      if (lastFrame) draw(lastFrame)
    }
    image.onerror = () => {
      if (activeGif !== image) return
      activeGifLoaded = false
    }
    image.decoding = 'async'
    image.src = gifUrls[state]
  }

  const draw = (frame: OrbFrame) => {
    if (disposed || (!loaded && !hdLoaded)) return
    const width = canvas.width
    const height = canvas.height
    context.clearRect(0, 0, width, height)

    const centerX = width / 2
    const centerY = height / 2
    const shortEdge = Math.min(width, height)
    if (!frame.reducedMotion && frame.rippleProgress !== null) {
      const progress = clamp(frame.rippleProgress, 0, 1)
      const radius = shortEdge * (0.29 + progress * 0.16)
      context.save()
      context.globalAlpha = clamp(frame.rippleAlpha, 0, 1) * (1 - progress * 0.35)
      context.strokeStyle = '#ffe2a8'
      context.lineWidth = Math.max(1, shortEdge * 0.008)
      context.beginPath()
      context.arc(centerX, centerY, radius, 0, Math.PI * 2)
      context.stroke()
      context.restore()
    }

    if (!frame.reducedMotion && frame.haloPulse > 0) {
      const gradient = context.createRadialGradient(
        centerX,
        centerY,
        shortEdge * 0.18,
        centerX,
        centerY,
        shortEdge * 0.48,
      )
      gradient.addColorStop(0, 'rgba(255, 190, 112, 0)')
      gradient.addColorStop(1, `rgba(255, 165, 105, ${clamp(frame.haloPulse, 0, 1) * 0.16})`)
      context.save()
      context.fillStyle = gradient
      context.fillRect(0, 0, width, height)
      context.restore()
    }

    const elapsedMs = Math.max(0, frame.nowMs - stateStartedAt)
    const petFrame = petFrameAt(frame.state, elapsedMs, frame.reducedMotion)
    const useGif = activeGifLoaded && !frame.reducedMotion
    const sourceImage = useGif ? activeGif! : hdLoaded ? hdSpriteSheet! : spriteSheet
    const sourceCellWidth = useGif || hdLoaded ? PET_HD_ATLAS.cellWidth : PET_ATLAS.cellWidth
    const sourceCellHeight = useGif || hdLoaded ? PET_HD_ATLAS.cellHeight : PET_ATLAS.cellHeight
    const sourceColumn = useGif ? 0 : petFrame.column
    const sourceRow = useGif ? 0 : hdLoaded ? PET_HD_ROWS[frame.state] : petFrame.row
    const level = frame.reducedMotion ? 0 : reactiveLevel(frame)
    const scale = 1 + level * 0.025
    const bob = level * height * 0.012
    const fitScale = Math.min(
      width * 0.94 / sourceCellWidth,
      height * 0.96 / sourceCellHeight,
    )
    const drawWidth = sourceCellWidth * fitScale * scale
    const drawHeight = sourceCellHeight * fitScale * scale

    context.save()
    context.imageSmoothingEnabled = true
    context.imageSmoothingQuality = 'high'
    context.globalAlpha = frame.state === 'ended' ? 0.82 : 1
    context.filter = `brightness(${1 + level * 0.045}) drop-shadow(0 ${Math.max(2, height * 0.012)}px ${Math.max(4, height * 0.025)}px rgba(65, 20, 35, 0.28))`
    context.drawImage(
      sourceImage,
      sourceColumn * sourceCellWidth,
      sourceRow * sourceCellHeight,
      sourceCellWidth,
      sourceCellHeight,
      centerX - drawWidth / 2,
      centerY - drawHeight / 2 - bob,
      drawWidth,
      drawHeight,
    )
    context.restore()
  }

  spriteSheet.onload = () => {
    if (disposed) return
    if (
      spriteSheet.naturalWidth !== PET_ATLAS.width
      || spriteSheet.naturalHeight !== PET_ATLAS.height
    ) {
      loadFailed = true
      return
    }
    loaded = true
    if (lastFrame) draw(lastFrame)
  }
  spriteSheet.onerror = () => { loadFailed = true }
  spriteSheet.decoding = 'async'
  spriteSheet.src = spriteSheetUrl

  if (hdSpriteSheet && hdSpriteSheetUrl) {
    hdSpriteSheet.onload = () => {
      if (disposed) return
      if (
        hdSpriteSheet.naturalWidth !== PET_HD_ATLAS.width
        || hdSpriteSheet.naturalHeight !== PET_HD_ATLAS.height
      ) return
      hdLoaded = true
      if (lastFrame) draw(lastFrame)
    }
    hdSpriteSheet.onerror = () => {
      // The validated v2 atlas remains the non-blocking fallback.
    }
    hdSpriteSheet.decoding = 'async'
    hdSpriteSheet.src = hdSpriteSheetUrl
  }

  const resize = (width: number, height: number, pixelRatio: number) => {
    if (disposed) return
    const ratio = clamp(pixelRatio, 1, 2)
    canvas.width = Math.max(1, Math.round(clamp(width, 1, 4096 / ratio) * ratio))
    canvas.height = Math.max(1, Math.round(clamp(height, 1, 4096 / ratio) * ratio))
    if (lastFrame) draw(lastFrame)
  }

  const update = (frame: OrbFrame) => {
    if (disposed) return
    if (loadFailed && !hdLoaded) throw new Error('pet_spritesheet_unavailable')
    const nowMs = clamp(frame.nowMs, 0, Number.MAX_SAFE_INTEGER)
    const gifState = PET_GIF_STATES[frame.state]
    if (previousState !== frame.state || nowMs < previousNowMs) {
      previousState = frame.state
      stateStartedAt = nowMs
    }
    activateGif(gifState)
    previousNowMs = nowMs
    lastFrame = { ...frame, nowMs }
    draw(lastFrame)
  }

  const dispose = () => {
    if (disposed) return
    disposed = true
    spriteSheet.onload = null
    spriteSheet.onerror = null
    if (hdSpriteSheet) {
      hdSpriteSheet.onload = null
      hdSpriteSheet.onerror = null
    }
    if (activeGif) {
      activeGif.onload = null
      activeGif.onerror = null
    }
    activeGif = null
    lastFrame = null
    context.clearRect(0, 0, canvas.width, canvas.height)
  }

  resize(canvas.clientWidth, canvas.clientHeight, 1)
  return { update, resize, dispose }
}
