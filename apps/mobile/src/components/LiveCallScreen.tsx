import {
  CameraRotate,
  Microphone,
  MicrophoneSlash,
  PhoneDisconnect,
  X,
} from '@phosphor-icons/react'
import { useEffect, useState, type RefObject } from 'react'
import { assetBlob } from '../api'
import { mapSessionState } from '../live/motion'
import type {
  RealtimeMode,
  ResponseArtifact,
  SessionState,
} from '../realtime/RealtimeSession'
import '../live/LiveCall.css'
import { LiveCaption } from './LiveCaption'
import { LiveOrb } from './LiveOrb'

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
    void assetBlob(server, accessToken, artifact.content_url)
      .then((blob) => {
        if (!active) return
        objectUrl = URL.createObjectURL(blob)
        setSource(objectUrl)
      })
      .catch(() => {
        if (active) setSource('')
      })
    return () => {
      active = false
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [accessToken, artifact.content_url, server])

  if (!source) return <div className="live-artifact-placeholder" aria-hidden="true" />
  return <img src={source} alt={artifact.caption || '实时生成的画面'} />
}

export type LiveCallScreenProps = {
  mode: RealtimeMode
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
  server: string
  accessToken: string
  videoRef: RefObject<HTMLVideoElement | null>
  captureCanvasRef: RefObject<HTMLCanvasElement | null>
  onToggleMute(): void
  onFlipCamera(): Promise<void>
  onLeave(): Promise<void>
}

export function LiveCallScreen({
  mode,
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
  server,
  accessToken,
  videoRef,
  captureCanvasRef,
  onToggleMute,
  onFlipCamera,
  onLeave,
}: LiveCallScreenProps) {
  const videoMode = mode === 'video'
  const statusClass = state === 'error'
    ? 'is-error'
    : state === 'speaking'
      ? 'is-speaking'
      : state === 'idle' || state === 'ended'
        ? ''
        : 'is-live'
  const stateDetail = state === 'using_tool' && toolStatus
    ? toolStatus
    : stateLabels[state]

  return (
    <section className={`call-screen live-call-screen ${videoMode ? 'has-video' : 'has-audio'}`}>
      <video
        ref={videoRef}
        className="camera-preview"
        autoPlay
        muted
        playsInline
      />
      <canvas ref={captureCanvasRef} hidden />
      {videoMode && <div className="camera-scrim" aria-hidden="true" />}

      <header className="call-header">
        <span className="call-mode">{videoMode ? '视频' : '语音'} · 智能响应</span>
        <div className={`call-status ${statusClass}`} role="status">
          <span aria-hidden="true" />
          <strong>{stateDetail}</strong>
          <small aria-hidden="true">{formatDuration(elapsed)}</small>
        </div>
        {videoMode ? (
          <button
            className="icon-button call-icon"
            type="button"
            aria-label="切换摄像头"
            onClick={() => void onFlipCamera()}
          >
            <CameraRotate />
          </button>
        ) : <span className="header-spacer" />}
      </header>

      <div className="live-stage">
        {!videoMode && (
          <div className="live-orb-wrap">
            <LiveOrb
              state={mapSessionState(state)}
              inputLevel={inputLevel}
              outputLevel={outputLevel}
            />
          </div>
        )}

        <div className="live-feedback">
          <span className="live-state-label">{stateDetail}</span>
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
