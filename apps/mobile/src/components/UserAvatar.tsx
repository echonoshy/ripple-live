import { useEffect, useState } from 'react'
import { assetBlob } from '../api'
import { avatarInitial } from '../avatar'

export function UserAvatar({
  server,
  token,
  email,
  avatarUrl,
  className = '',
}: {
  server: string
  token: string
  email: string
  avatarUrl: string | null
  className?: string
}) {
  const [source, setSource] = useState('')

  useEffect(() => {
    setSource('')
    if (!avatarUrl) return
    const controller = new AbortController()
    let active = true
    let objectUrl = ''
    void assetBlob(server, token, avatarUrl, controller.signal)
      .then((blob) => {
        if (!active) return
        objectUrl = URL.createObjectURL(blob)
        setSource(objectUrl)
      })
      .catch(() => {
        if (active) setSource('')
      })
    return () => {
      active = false
      controller.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [avatarUrl, server, token])

  const classes = `user-avatar ${className}`.trim()
  if (!source) {
    return <span className={classes} aria-hidden="true">{avatarInitial(email)}</span>
  }
  return <img className={classes} src={source} alt="" aria-hidden="true" />
}
