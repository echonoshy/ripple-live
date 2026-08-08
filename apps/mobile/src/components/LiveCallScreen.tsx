import {
  CameraRotate,
  CaretDown,
  Microphone,
  MicrophoneSlash,
  VideoCamera,
  VideoCameraSlash,
  X,
} from '@phosphor-icons/react'
import { useEffect, useState, type CSSProperties, type RefObject } from 'react'
import { assetBlob } from '../api'
import { mapSessionState } from '../live/motion'
import type { CameraPhase } from '../live/cameraOrchestration'
import type { RippleSignal } from '../live/ripple'
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
  preparing: '准备中',
  listening: '我在听',
  thinking: '想一想',
  using_tool: '处理中',
  speaking: '',
  ended: '通话已结束',
  error: '连接断开',
}

function formatDuration(seconds: number) {
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return `${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}`
}

function clampLevel(level: number) {
  if (!Number.isFinite(level)) return 0
  return Math.min(1, Math.max(0, level))
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
  cameraControlReady: boolean
  frameRequestActive: boolean
  state: SessionState
  elapsed: number
  muted: boolean
  inputLevel: number
  outputLevel: number
  rippleSignal: RippleSignal | null
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
  cameraControlReady,
  frameRequestActive,
  state,
  elapsed,
  muted,
  inputLevel,
  outputLevel,
  rippleSignal,
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
  const cameraStatus = cameraPhase === 'opening'
      ? '正在开启镜头'
      : cameraPhase === 'on'
        ? '镜头已开启'
        : stateDetail
  const orbStyle = {
    '--live-input-scale': 0.98 + clampLevel(inputLevel) * 0.065,
    '--live-output-scale': 0.96 + clampLevel(outputLevel) * 0.115,
  } as CSSProperties
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
        <button
          className="icon-button call-icon call-collapse"
          type="button"
          aria-label="收起通话"
          onClick={() => void onLeave()}
        >
          <CaretDown aria-hidden="true" />
        </button>
        <div className="call-title">
          <strong>Ripple</strong>
          <span aria-label={`通话时长 ${formatDuration(elapsed)}`}>
            {formatDuration(elapsed)}
          </span>
        </div>
        {cameraPhase === 'on' ? (
          <button
            className="icon-button call-icon"
            type="button"
            aria-label="切换摄像头"
            onClick={() => { void onFlipCamera().catch(() => {}) }}
          >
            <CameraRotate aria-hidden="true" />
          </button>
        ) : <span className="header-spacer" />}
      </header>

      <div className="live-stage">
        <div className="live-orb-wrap" style={orbStyle}>
          <LiveOrb
            state={mapSessionState(state)}
            inputLevel={inputLevel}
            outputLevel={outputLevel}
            rippleSignal={rippleSignal}
          />
        </div>

        <div className="live-feedback">
          {cameraStatus && (
            <span className="live-state-label" role="status">
              {cameraStatus}
            </span>
          )}
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
        <button
          className={`control-button camera-control ${videoMode ? 'is-active' : ''}`}
          type="button"
          aria-label={
            !cameraControlReady
              ? '镜头尚未就绪'
              : cameraPhase === 'opening'
                ? '正在开启镜头'
                : cameraPhase === 'closing'
                  ? '正在关闭镜头'
                  : videoMode
                    ? '关闭镜头'
                    : cameraPhase === 'error'
                      ? '重试镜头'
                      : '开启镜头'
          }
          disabled={!cameraControlReady || cameraBusy}
          onClick={() => { void onToggleCamera().catch(() => {}) }}
        >
          {videoMode
            ? <VideoCameraSlash aria-hidden="true" />
            : <VideoCamera aria-hidden="true" />}
        </button>
        <button
          className={`control-button ${muted ? 'is-active' : ''}`}
          type="button"
          aria-label={muted ? '取消静音' : '静音'}
          onClick={onToggleMute}
        >
          {muted
            ? <MicrophoneSlash aria-hidden="true" />
            : <Microphone aria-hidden="true" />}
        </button>
        <button
          className="end-button"
          type="button"
          aria-label="结束通话"
          onClick={() => void onLeave()}
        >
          <X weight="bold" aria-hidden="true" />
        </button>
      </footer>
    </section>
  )
}
