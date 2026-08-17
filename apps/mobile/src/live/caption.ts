import type { SessionState } from '../realtime/RealtimeSession'

export type UserCaptionState = {
  text: string
  responseActive: boolean
}

export const emptyUserCaption: UserCaptionState = {
  text: '',
  responseActive: false,
}

export function nextUserCaption(
  current: UserCaptionState,
  state: SessionState,
  userText: string,
): UserCaptionState {
  if (state === 'thinking' || state === 'using_tool') {
    return {
      text: userText || current.text,
      responseActive: true,
    }
  }

  if (state === 'speaking') {
    return { text: '', responseActive: true }
  }

  if (state === 'listening') {
    if (current.responseActive) return emptyUserCaption
    return { text: userText, responseActive: false }
  }

  return emptyUserCaption
}
