import { MOTION_TIMING } from './motion'
import type { SessionState } from '../realtime/RealtimeSession'

type CaptionTimer = {
  setTimeout(callback: () => void, delay: number): number
  clearTimeout(timer: number): void
}

export type CaptionSnapshot = {
  source: 'user' | 'assistant'
  userText: string
  assistantText: string
}

export function captionTextForState(
  state: SessionState,
  userText: string,
  assistantText: string,
) {
  return state === 'speaking' ? assistantText : userText
}

export function scheduleCaptionClear(
  callback: () => void,
  timer: CaptionTimer = window,
) {
  const timerId = timer.setTimeout(callback, MOTION_TIMING.captionHoldMs)
  return () => timer.clearTimeout(timerId)
}

export function nextCaptionText(
  previous: CaptionSnapshot,
  current: CaptionSnapshot,
) {
  const text = current.source === 'assistant'
    ? current.assistantText
    : current.userText
  if (current.source === previous.source) return text

  const sourceTextChanged = current.source === 'assistant'
    ? current.assistantText !== previous.assistantText
    : current.userText !== previous.userText
  return sourceTextChanged ? text : ''
}
