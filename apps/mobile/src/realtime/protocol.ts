export type RealtimeMode = 'audio' | 'video'

export const REALTIME_PROTOCOL_VERSION = 3

const clientBuild =
  typeof __RIPPLE_CLIENT_BUILD__ === 'string'
    ? __RIPPLE_CLIENT_BUILD__
    : '0.1.1-test'

export function createSessionStart(mode: RealtimeMode) {
  return {
    type: 'session.start',
    protocol_version: REALTIME_PROTOCOL_VERSION,
    client_build: clientBuild,
    mode,
  }
}

export function createRequestedFrameEvents(
  responseId: string,
  frame: string | null,
  capturedAt: number,
) {
  const events: Array<Record<string, unknown>> = []
  if (frame) {
    events.push({
      type: 'input.video.frame',
      response_id: responseId,
      image: frame,
      mime_type: 'image/jpeg',
      captured_at: capturedAt,
    })
  }
  events.push({ type: 'input.video.commit', response_id: responseId })
  return events
}
