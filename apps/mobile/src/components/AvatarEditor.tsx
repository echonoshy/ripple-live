import { Minus, Plus, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { cropSourceRect } from '../avatar'

const PREVIEW_SIZE = 280
const OUTPUT_SIZE = 512

type Size = { width: number; height: number }
type Offset = { x: number; y: number }

function clampOffset(offset: Offset, size: Size, zoom: number): Offset {
  if (!size.width || !size.height) return { x: 0, y: 0 }
  const baseScale = Math.max(PREVIEW_SIZE / size.width, PREVIEW_SIZE / size.height)
  const maxX = Math.max(0, (size.width * baseScale * zoom - PREVIEW_SIZE) / 2)
  const maxY = Math.max(0, (size.height * baseScale * zoom - PREVIEW_SIZE) / 2)
  return {
    x: Math.max(-maxX, Math.min(maxX, offset.x)),
    y: Math.max(-maxY, Math.min(maxY, offset.y)),
  }
}

export function AvatarEditor({
  file,
  busy,
  error,
  onCancel,
  onSave,
}: {
  file: File
  busy: boolean
  error: string
  onCancel(): void
  onSave(blob: Blob): Promise<void>
}) {
  const [source, setSource] = useState('')
  const [size, setSize] = useState<Size>({ width: 0, height: 0 })
  const [zoom, setZoom] = useState(1)
  const [offset, setOffset] = useState<Offset>({ x: 0, y: 0 })
  const [localError, setLocalError] = useState('')
  const imageRef = useRef<HTMLImageElement>(null)
  const dragRef = useRef<{
    pointerId: number
    startX: number
    startY: number
    offset: Offset
  } | null>(null)

  useEffect(() => {
    const objectUrl = URL.createObjectURL(file)
    setSource(objectUrl)
    setSize({ width: 0, height: 0 })
    setZoom(1)
    setOffset({ x: 0, y: 0 })
    setLocalError('')
    return () => URL.revokeObjectURL(objectUrl)
  }, [file])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) onCancel()
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [busy, onCancel])

  const imageStyle = useMemo(() => {
    if (!size.width || !size.height) return undefined
    const baseScale = Math.max(PREVIEW_SIZE / size.width, PREVIEW_SIZE / size.height)
    return {
      width: `${size.width * baseScale}px`,
      height: `${size.height * baseScale}px`,
      transform: `translate(-50%, -50%) translate(${offset.x}px, ${offset.y}px) scale(${zoom})`,
    }
  }, [offset.x, offset.y, size.height, size.width, zoom])

  function updateZoom(nextZoom: number) {
    setZoom(nextZoom)
    setOffset((current) => clampOffset(current, size, nextZoom))
  }

  function beginDrag(event: React.PointerEvent<HTMLDivElement>) {
    if (busy || !size.width) return
    event.currentTarget.setPointerCapture(event.pointerId)
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      offset,
    }
  }

  function moveDrag(event: React.PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== event.pointerId) return
    setOffset(clampOffset({
      x: drag.offset.x + event.clientX - drag.startX,
      y: drag.offset.y + event.clientY - drag.startY,
    }, size, zoom))
  }

  function endDrag(event: React.PointerEvent<HTMLDivElement>) {
    if (dragRef.current?.pointerId === event.pointerId) dragRef.current = null
  }

  async function save() {
    const image = imageRef.current
    if (!image || !size.width || !size.height) return
    setLocalError('')
    const source = cropSourceRect(size, zoom, offset)
    const canvas = document.createElement('canvas')
    canvas.width = OUTPUT_SIZE
    canvas.height = OUTPUT_SIZE
    const context = canvas.getContext('2d')
    if (!context) {
      setLocalError('无法处理这张图片，请换一张重试')
      return
    }
    context.drawImage(
      image,
      source.x,
      source.y,
      source.side,
      source.side,
      0,
      0,
      OUTPUT_SIZE,
      OUTPUT_SIZE,
    )
    const blob = await new Promise<Blob | null>((resolve) => {
      canvas.toBlob(resolve, 'image/jpeg', 0.86)
    })
    if (!blob) {
      setLocalError('无法生成头像，请换一张图片重试')
      return
    }
    await onSave(blob)
  }

  return (
    <div className="avatar-editor-backdrop" role="presentation">
      <section className="avatar-editor" role="dialog" aria-modal="true" aria-labelledby="avatar-editor-title">
        <header>
          <div>
            <h2 id="avatar-editor-title">调整头像</h2>
            <p>拖动图片并缩放到合适位置</p>
          </div>
          <button type="button" aria-label="关闭头像编辑" onClick={onCancel} disabled={busy}>
            <X aria-hidden="true" />
          </button>
        </header>

        <div
          className="avatar-crop-stage"
          onPointerDown={beginDrag}
          onPointerMove={moveDrag}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          {source ? (
            <img
              ref={imageRef}
              src={source}
              alt="待裁剪的头像"
              draggable="false"
              style={imageStyle}
              onLoad={(event) => {
                setSize({
                  width: event.currentTarget.naturalWidth,
                  height: event.currentTarget.naturalHeight,
                })
              }}
              onError={() => setLocalError('无法读取这张图片，请换一张重试')}
            />
          ) : null}
          <span className="avatar-crop-ring" aria-hidden="true" />
        </div>

        <label className="avatar-zoom-control">
          <Minus aria-hidden="true" />
          <span className="visually-hidden">头像缩放</span>
          <input
            type="range"
            min="1"
            max="3"
            step="0.01"
            value={zoom}
            onChange={(event) => updateZoom(Number(event.target.value))}
            disabled={busy || !size.width}
          />
          <Plus aria-hidden="true" />
        </label>

        {(error || localError) ? <p className="avatar-editor-error" role="alert">{error || localError}</p> : null}

        <div className="avatar-editor-actions">
          <button type="button" onClick={onCancel} disabled={busy}>取消</button>
          <button className="primary-action" type="button" onClick={() => void save()} disabled={busy || !size.width}>
            {busy ? '正在上传…' : '保存头像'}
          </button>
        </div>
      </section>
    </div>
  )
}
