export type CameraSwitchOutcome = 'switched' | 'stale' | 'failed'

const CAMERA_SWITCH_ERROR = '无法切换摄像头，请重试'

export function cameraErrorAfterSwitch(
  previous: string,
  outcome: CameraSwitchOutcome,
) {
  if (outcome === 'stale') return previous
  return outcome === 'failed' ? CAMERA_SWITCH_ERROR : ''
}

export function visibleCallError(
  sessionError: string,
  cameraError: string,
) {
  return sessionError || cameraError
}
