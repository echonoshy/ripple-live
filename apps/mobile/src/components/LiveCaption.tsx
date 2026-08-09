import { useEffect, useState } from 'react'
import {
  emptyUserCaption,
  nextUserCaption,
} from '../live/caption'
import type { SessionState } from '../realtime/RealtimeSession'

export type LiveCaptionProps = {
  userText: string
  assistantText: string
  state: SessionState
}

export function LiveCaption({ userText, state }: LiveCaptionProps) {
  const [caption, setCaption] = useState(emptyUserCaption)

  useEffect(() => {
    setCaption((current) => nextUserCaption(current, state, userText))
  }, [state, userText])

  return (
    <p
      className="live-caption"
      data-caption-source={caption.text ? 'user' : undefined}
      aria-live="polite"
      aria-atomic="true"
    >
      {caption.text}
    </p>
  )
}
