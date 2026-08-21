const AVATAR_PREVIEW_SIZE = 280

export function avatarInitial(email: string) {
  return email.trim().charAt(0).toLocaleUpperCase() || 'R'
}

export function cropSourceRect(
  size: { width: number; height: number },
  zoom: number,
  offset: { x: number; y: number },
) {
  const baseScale = Math.max(
    AVATAR_PREVIEW_SIZE / size.width,
    AVATAR_PREVIEW_SIZE / size.height,
  )
  const displayScale = baseScale * zoom
  const displayedWidth = size.width * displayScale
  const displayedHeight = size.height * displayScale
  return {
    x: Math.max(
      0,
      (displayedWidth / 2 - offset.x - AVATAR_PREVIEW_SIZE / 2) / displayScale,
    ),
    y: Math.max(
      0,
      (displayedHeight / 2 - offset.y - AVATAR_PREVIEW_SIZE / 2) / displayScale,
    ),
    side: AVATAR_PREVIEW_SIZE / displayScale,
  }
}
