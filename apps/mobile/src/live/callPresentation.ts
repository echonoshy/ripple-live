import type { CameraPhase } from './cameraOrchestration'
import type { SessionState } from '../realtime/RealtimeSession'

const stateLabels: Record<SessionState, string> = {
  idle: '准备就绪',
  connecting: '正在连接',
  preparing: '准备中',
  listening: '我在听',
  thinking: '想一想',
  using_tool: '处理中',
  speaking: '',
  ended: '通话已结束',
  error: '连接断开',
}

export function liveCallLabels(
  state: SessionState,
  cameraPhase: CameraPhase,
  toolStatus: string,
) {
  const primary = state === 'using_tool' && toolStatus
    ? toolStatus
    : stateLabels[state]
  const camera = cameraPhase === 'opening'
    ? '正在开启镜头'
    : cameraPhase === 'on'
      ? '镜头已开启'
      : ''
  return { primary, camera }
}

export type CameraHeaderAction = {
  kind: 'toggle' | 'flip'
  label: string
  disabled: boolean
}

export function cameraHeaderAction(
  phase: CameraPhase,
  previewVisible: boolean,
  ready: boolean,
): CameraHeaderAction {
  if (!ready) {
    return { kind: 'toggle', label: '镜头尚未就绪', disabled: true }
  }
  switch (phase) {
    case 'off':
      return { kind: 'toggle', label: '开启镜头', disabled: false }
    case 'opening':
      return { kind: 'toggle', label: '正在开启镜头', disabled: true }
    case 'on':
      return { kind: 'flip', label: '切换摄像头', disabled: false }
    case 'closing':
      return { kind: 'toggle', label: '正在关闭镜头', disabled: true }
    case 'error':
      return {
        kind: 'toggle',
        label: previewVisible ? '重试关闭镜头' : '重试镜头',
        disabled: false,
      }
  }
}
