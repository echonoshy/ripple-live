import {
  CameraRotate,
  Microphone,
  MicrophoneSlash,
  PhoneDisconnect,
  VideoCamera,
  VideoCameraSlash,
  X,
} from '@phosphor-icons/react'
import { useEffect, useState, type RefObject } from 'react'
import { assetBlob } from '../api'
import { mapSessionState } from '../live/motion'
import type { CameraPhase } from '../live/cameraOrchestration'
import type {
  RealtimeMode,
  ResponseArtifact,
  SessionState,
} from '../realtime/RealtimeSession'
import type { LiveResult } from '../realtime/toolResults'
import '../live/LiveCall.css'
import { LiveCaption } from './LiveCaption'
import { LiveOrb } from './LiveOrb'
import { LiveResultSheet } from './LiveResultSheet'

const stateLabels: Record<SessionState, string> = {
  idle: '准备就绪',
  connecting: '正在连接',
  preparing: '正在准备模型',
  listening: '正在聆听',
  thinking: '正在思考',
  using_tool: '正在使用工具',
  speaking: '正在回答',
  ended: '通话已结束',
  error: '连接异常',
}

function formatDuration(seconds: number) {
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return `${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}`
}

function AuthenticatedArtifact({
  artifact,
  server,
  accessToken,
}: {
  artifact: ResponseArtifact
  server: string
  accessToken: string
}) {
  const [source, setSource] = useState('')

  useEffect(() => {
    let active = true
    let objectUrl = ''
    const controller = new AbortController()
    void assetBlob(server, accessToken, artifact.content_url, controller.signal)
      .then((blob) => {
        if (!active) return
        objectUrl = URL.createObjectURL(blob)
        setSource(objectUrl)
      })
      .catch((error: unknown) => {
        if (error instanceof Error && error.name === 'AbortError') return
        if (active) setSource('')
      })
    return () => {
      active = false
      controller.abort()
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [accessToken, artifact.content_url, server])

  if (!source) return <div className="live-artifact-placeholder" aria-hidden="true" />
  return <img src={source} alt={artifact.caption || '实时生成的画面'} />
}

export type LiveCallScreenProps = {
  mode: RealtimeMode
  cameraPhase: CameraPhase
  cameraPreviewVisible: boolean
  frameRequestActive: boolean
  state: SessionState
  elapsed: number
  muted: boolean
  inputLevel: number
  outputLevel: number
  userText: string
  assistantText: string
  toolStatus: string
  errorMessage: string
  artifacts: ResponseArtifact[]
  results: LiveResult[]
  server: string
  accessToken: string
  videoRef: RefObject<HTMLVideoElement | null>
  captureCanvasRef: RefObject<HTMLCanvasElement | null>
  onToggleMute(): void
  onToggleCamera(): Promise<void>
  onFlipCamera(): Promise<void>
  onDismissResult(callId: string): void
  onLeave(): Promise<void>
}

export function LiveCallScreen({
  mode,
  cameraPhase,
  cameraPreviewVisible,
  frameRequestActive,
  state,
  elapsed,
  muted,
  inputLevel,
  outputLevel,
  userText,
  assistantText,
  toolStatus,
  errorMessage,
  artifacts,
  results,
  server,
  accessToken,
  videoRef,
  captureCanvasRef,
  onToggleMute,
  onToggleCamera,
  onFlipCamera,
  onDismissResult,
  onLeave,
}: LiveCallScreenProps) {
  const videoMode = cameraPreviewVisible
  const cameraBusy = cameraPhase === 'opening' || cameraPhase === 'closing'
  const stateDetail = state === 'using_tool' && toolStatus
    ? toolStatus
    : stateLabels[state]
  const cameraStatus = frameRequestActive && cameraPhase === 'on'
    ? '正在识别'
    : cameraPhase === 'opening'
      ? '正在开启镜头'
      : cameraPhase === 'on'
        ? '镜头已开启'
        : cameraPhase === 'closing'
          ? '正在关闭镜头'
          : cameraPhase === 'error' && cameraPreviewVisible
            ? '镜头状态待同步'
            : stateDetail
  const statusClass = state === 'error'
    ? 'is-error'
    : state === 'speaking'
      ? 'is-speaking'
      : state === 'idle' || state === 'ended'
        ? ''
        : 'is-live'
  return (
    <section
      className={`call-screen live-call-screen ${videoMode ? 'has-video' : 'has-audio'} server-${mode} camera-phase-${cameraPhase} ${results.length > 0 ? 'has-results' : ''}`}
    >
      <div className="camera-layer" aria-hidden={!videoMode}>
        <video
          ref={videoRef}
          className="camera-preview"
          autoPlay
          muted
          playsInline
        />
        <div className="camera-scrim" aria-hidden="true" />
      </div>
      <canvas ref={captureCanvasRef} hidden />
      {frameRequestActive && cameraPhase === 'on' && videoMode && (
        <div className="camera-focus-frame" aria-hidden="true">
          <span />
        </div>
      )}

      <header className="call-header">
        <span className="call-mode">
          {cameraPhase === 'opening'
            ? '正在开启镜头'
            : cameraPhase === 'on'
              ? '镜头已开启'
              : cameraPhase === 'closing'
                ? '正在关闭镜头'
                : cameraPhase === 'error' && videoMode
                  ? '镜头待同步'
                  : '语音'} · 智能响应
        </span>
        <div className={`call-status ${statusClass}`} role="status">
          <span aria-hidden="true" />
          <strong>{stateDetail}</strong>
          <small aria-hidden="true">{formatDuration(elapsed)}</small>
        </div>
        {cameraPhase === 'on' ? (
          <button
            className="icon-button call-icon"
            type="button"
            aria-label="切换摄像头"
            onClick={() => { void onFlipCamera().catch(() => {}) }}
          >
            <CameraRotate />
          </button>
        ) : <span className="header-spacer" />}
      </header>

      <div className="live-stage">
        <div className="live-orb-wrap">
          <LiveOrb
            state={mapSessionState(state)}
            inputLevel={inputLevel}
            outputLevel={outputLevel}
          />
        </div>

        <div className="live-feedback">
          <span className="live-state-label">{cameraStatus}</span>
          <LiveCaption
            userText={userText}
            assistantText={assistantText}
            state={state}
          />
          {errorMessage && (
            <div className="live-error" role="alert">
              <X weight="bold" aria-hidden="true" />
              <span>{errorMessage}</span>
            </div>
          )}
        </div>
      </div>

      {(results.length > 0 || artifacts.length > 0) && (
        <div className="live-output-tray">
          <LiveResultSheet results={results} onDismiss={onDismissResult} />
          {artifacts.length > 0 && (
            <aside className="live-artifact-sheet" aria-label="实时生成的画面">
              <div className="live-artifact-strip">
                {artifacts.map((artifact) => (
                  <AuthenticatedArtifact
                    key={artifact.id}
                    artifact={artifact}
                    server={server}
                    accessToken={accessToken}
                  />
                ))}
              </div>
            </aside>
          )}
        </div>
      )}

      <footer className="call-controls">
        <div className="control-item">
          <button
            className={`control-button ${muted ? 'is-active' : ''}`}
            type="button"
            aria-label={muted ? '取消静音' : '静音'}
            onClick={onToggleMute}
          >
            {muted ? <MicrophoneSlash /> : <Microphone />}
          </button>
          <span>{muted ? '取消静音' : '静音'}</span>
        </div>
        <div className="control-item">
          <button
            className={`control-button camera-control ${videoMode ? 'is-active' : ''}`}
            type="button"
            aria-label={
              cameraBusy
                ? cameraStatus
                : videoMode
                  ? '关闭镜头'
                  : cameraPhase === 'error'
                    ? '重试镜头'
                    : '开启镜头'
            }
            disabled={cameraBusy}
            onClick={() => { void onToggleCamera().catch(() => {}) }}
          >
            {videoMode ? <VideoCameraSlash /> : <VideoCamera />}
          </button>
          <span>
            {cameraPhase === 'opening'
              ? '开启中'
              : cameraPhase === 'closing'
                ? '关闭中'
                : cameraPhase === 'error'
                  ? '重试'
                  : videoMode
                    ? '关闭镜头'
                    : '开启镜头'}
          </span>
        </div>
        <div className="control-item">
          <button
            className="end-button"
            type="button"
            aria-label="结束通话"
            onClick={() => void onLeave()}
          >
            <PhoneDisconnect weight="fill" />
          </button>
          <span>结束</span>
        </div>
      </footer>
    </section>
  )
}
