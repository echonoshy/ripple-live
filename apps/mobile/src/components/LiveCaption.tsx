import { useEffect, useRef, useState } from 'react'
import {
  captionTextForState,
  nextCaptionText,
  scheduleCaptionClear,
  type CaptionSnapshot,
} from '../live/caption'
import type { SessionState } from '../realtime/RealtimeSession'

export type LiveCaptionProps = {
  userText: string
  assistantText: string
  state: SessionState
}

export function LiveCaption({ userText, assistantText, state }: LiveCaptionProps) {
  const source: CaptionSnapshot['source'] = state === 'speaking' ? 'assistant' : 'user'
  const text = captionTextForState(state, userText, assistantText)
  const [visible, setVisible] = useState('')
  const previous = useRef({ source, userText, assistantText })

  useEffect(() => {
    const current = { source, userText, assistantText }
    const next = source === previous.current.source
      ? text
      : nextCaptionText(previous.current, current)
    previous.current = current
    setVisible(next)
    if (!next) return
    return scheduleCaptionClear(() => setVisible(''))
  }, [assistantText, source, text, userText])

  return (
    <div className="live-caption" aria-live="polite" aria-atomic="true">
      {visible}
    </div>
  )
}
