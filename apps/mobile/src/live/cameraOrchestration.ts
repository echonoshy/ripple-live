import type { CameraEnableResult, CameraFacingMode } from '../media/CameraController'
import type { RealtimeMode } from '../realtime/RealtimeSession'

export type CameraPhase = 'off' | 'opening' | 'on' | 'closing' | 'error'
export type CameraRecovery = 'open' | 'close' | 'audio' | null

export type CameraSnapshot = {
  phase: CameraPhase
  previewVisible: boolean
  recovery: CameraRecovery
  serverMode: RealtimeMode | 'unknown'
}

export type CameraTransactionResult = CameraPhase | 'stale'

type CameraOrchestratorDependencies = {
  enableCamera(facingMode: CameraFacingMode): Promise<CameraEnableResult>
  disableCamera(): void
  setMode(mode: RealtimeMode): Promise<void>
  waitForTransition(): Promise<void>
  onSnapshot(snapshot: CameraSnapshot): void
  onError?(message: string): void
}

const initialSnapshot = (): CameraSnapshot => ({
  phase: 'off',
  previewVisible: false,
  recovery: null,
  serverMode: 'audio',
})

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback
}

/**
 * Owns camera/server-mode transactions for one live call. Invalidation is the
 * generation boundary used by leave, logout and session replacement.
 */
export function createCameraOrchestrator(
  dependencies: CameraOrchestratorDependencies,
) {
  let generation = 0
  let snapshot = initialSnapshot()
  let active: Promise<CameraTransactionResult> | null = null

  const publish = (next: CameraSnapshot) => {
    snapshot = next
    dependencies.onSnapshot({ ...next })
  }
  const owns = (owner: number) => owner === generation
  const finish = (
    owner: number,
    request: Promise<CameraTransactionResult>,
  ) => {
    if (owns(owner) && active === request) active = null
  }

  const open = (facingMode: CameraFacingMode) => {
    if (active) return active
    if (snapshot.previewVisible) return Promise.resolve(snapshot.phase)
    const owner = ++generation
    publish({
      phase: 'opening',
      previewVisible: false,
      recovery: null,
      serverMode: snapshot.serverMode === 'unknown' ? 'unknown' : 'audio',
    })
    const request = (async (): Promise<CameraTransactionResult> => {
      try {
        const enabled = await dependencies.enableCamera(facingMode)
        if (!owns(owner) || enabled === 'stale') return 'stale'
      } catch (error) {
        if (!owns(owner)) return 'stale'
        dependencies.disableCamera()
        dependencies.onError?.(
          errorMessage(error, '无法开启镜头，请检查摄像头权限后重试'),
        )
        publish({
          phase: 'error',
          previewVisible: false,
          recovery: 'open',
          serverMode: 'audio',
        })
        return 'error'
      }

      try {
        await dependencies.setMode('video')
        if (!owns(owner)) return 'stale'
        publish({
          phase: 'on',
          previewVisible: true,
          recovery: null,
          serverMode: 'video',
        })
        return 'on'
      } catch (videoError) {
        if (!owns(owner)) return 'stale'
        try {
          // A timeout can mean the server switched but its acknowledgement was
          // lost. Confirm audio before releasing the local camera.
          await dependencies.setMode('audio')
          if (!owns(owner)) return 'stale'
          dependencies.disableCamera()
          dependencies.onError?.(
            `镜头未能与会话同步：${errorMessage(videoError, '切换视频失败')}`,
          )
          publish({
            phase: 'error',
            previewVisible: false,
            recovery: 'open',
            serverMode: 'audio',
          })
          return 'error'
        } catch (audioError) {
          if (!owns(owner)) return 'stale'
          dependencies.onError?.(
            `镜头状态未同步，请重试关闭：${errorMessage(audioError, '恢复语音失败')}`,
          )
          publish({
            phase: 'error',
            previewVisible: true,
            recovery: 'close',
            serverMode: 'unknown',
          })
          return 'error'
        }
      }
    })()
    active = request
    void request.then(
      () => finish(owner, request),
      () => finish(owner, request),
    )
    return request
  }

  const close = () => {
    if (active) return active
    if (!snapshot.previewVisible) return Promise.resolve(snapshot.phase)
    const owner = ++generation
    publish({ ...snapshot, phase: 'closing', recovery: null })
    const request = (async (): Promise<CameraTransactionResult> => {
      try {
        await dependencies.setMode('audio')
      } catch (error) {
        if (!owns(owner)) return 'stale'
        dependencies.onError?.(
          `无法关闭镜头，请重试：${errorMessage(error, '切换语音失败')}`,
        )
        publish({
          phase: 'error',
          previewVisible: true,
          recovery: 'close',
          serverMode: 'unknown',
        })
        return 'error'
      }
      if (!owns(owner)) return 'stale'
      publish({
        phase: 'closing',
        previewVisible: false,
        recovery: null,
        serverMode: 'audio',
      })
      await dependencies.waitForTransition()
      if (!owns(owner)) return 'stale'
      dependencies.disableCamera()
      publish(initialSnapshot())
      return 'off'
    })()
    active = request
    void request.then(
      () => finish(owner, request),
      () => finish(owner, request),
    )
    return request
  }

  const interrupt = () => {
    if (snapshot.recovery === 'audio' && active) return active
    const prior = active
    const owner = ++generation
    publish({
      phase: 'error',
      previewVisible: false,
      recovery: 'audio',
      serverMode: 'unknown',
    })
    const request = (async (): Promise<CameraTransactionResult> => {
      // RealtimeSession permits one mode transaction. A camera track can end
      // after video was sent but before its ACK/timeout. Let that transaction
      // settle, then always confirm audio while this call still owns the work.
      if (prior) {
        try {
          await prior
        } catch {
          // The corrective audio request below is authoritative.
        }
      }
      if (!owns(owner)) return 'stale'
      try {
        await dependencies.setMode('audio')
        if (!owns(owner)) return 'stale'
        dependencies.onError?.('镜头已中断，已继续语音通话')
        publish(initialSnapshot())
        return 'off'
      } catch (error) {
        if (!owns(owner)) return 'stale'
        dependencies.onError?.(
          `镜头已中断，模式仍待同步：${errorMessage(error, '恢复语音失败')}`,
        )
        return 'error'
      }
    })()
    active = request
    void request.then(
      () => finish(owner, request),
      () => finish(owner, request),
    )
    return request
  }

  const retry = (facingMode: CameraFacingMode) => {
    switch (snapshot.recovery) {
      case 'close':
        return close()
      case 'audio':
        return interrupt()
      case 'open':
      default:
        return open(facingMode)
    }
  }

  dependencies.onSnapshot({ ...snapshot })
  return {
    close,
    current: () => ({ ...snapshot }),
    interrupt,
    invalidate() {
      generation += 1
      active = null
      publish(initialSnapshot())
    },
    open,
    retry,
  }
}
