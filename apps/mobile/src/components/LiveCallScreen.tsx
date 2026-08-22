import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  SwitchCamera as CameraRotate,
  Mic as Microphone,
  MicOff as MicrophoneSlash,
  Phone,
  Video as VideoCamera,
  VideoOff as VideoCameraSlash,
  X,
} from 'lucide-react'
import { useEffect, useState, type RefObject } from 'react'
import { assetBlob } from '../api'
import { mapSessionState } from '../live/motion'
import type { CameraPhase } from '../live/cameraOrchestration'
import { liveCallLabels } from '../live/callPresentation'
import type { RippleSignal, RippleSignalId } from '../live/ripple'
import type {
  RealtimeMode,
  ResponseArtifact,
  SessionState,
  LiveTranscriptTurn,
} from '../realtime/RealtimeSession'
import type { LiveResult } from '../realtime/toolResults'
import '../live/LiveCall.css'
import { LiveCaption } from './LiveCaption'
import { LiveOrb } from './LiveOrb'
import { LiveResultSheet } from './LiveResultSheet'

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
  cameraControlReady: boolean
  frameRequestActive: boolean
  state: SessionState
  elapsed: number
  muted: boolean
  inputLevel: number
  outputLevel: number
  rippleSignals: readonly RippleSignal[]
  onRippleSignalsConsumed(signalId: RippleSignalId): void
  userText: string
  assistantText: string
  toolStatus: string
  errorMessage: string
  artifacts: ResponseArtifact[]
  results: LiveResult[]
  transcript: LiveTranscriptTurn[]
  transcriptError: string
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
  rippleSignals,
  onRippleSignalsConsumed,
  userText,
  assistantText,
  toolStatus,
  errorMessage,
  artifacts,
  results,
  transcript,
  transcriptError,
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
  const [transcriptExpanded, setTranscriptExpanded] = useState(false)
  const videoMode = cameraPreviewVisible
  const cameraBusy = cameraPhase === 'opening' || cameraPhase === 'closing'
  const hasOutput = results.length > 0 || artifacts.length > 0
  const labels = liveCallLabels(state, cameraPhase, toolStatus)
  const visibleErrorMessage = errorMessage === 'Permission dismissed'
    ? '未授予麦克风或摄像头权限'
    : errorMessage
  return (
    <section
      className={`call-screen live-call-screen ${videoMode ? 'has-video' : 'has-audio'} server-${mode} camera-phase-${cameraPhase} ${hasOutput ? 'has-results' : ''}`}
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
          className="icon-button call-icon call-back"
          type="button"
          aria-label="结束并返回"
          onClick={() => void onLeave()}
        >
          <ArrowLeft aria-hidden="true" />
        </button>
        <div className="call-title">
          <strong><i aria-hidden="true" />正在陪伴</strong>
          <span aria-label={`通话时长 ${formatDuration(elapsed)}`}>{formatDuration(elapsed)}</span>
        </div>
        {videoMode ? (
          <button
            className="icon-button call-icon"
            type="button"
            aria-label="切换前后摄像头"
            disabled={!cameraControlReady || cameraBusy}
            onClick={() => void onFlipCamera().catch(() => {})}
          >
            <CameraRotate aria-hidden="true" />
          </button>
        ) : (
          <span className="call-header-spacer" />
        )}
      </header>

      <div className="live-stage">
        <div className="live-orb-wrap">
          <LiveOrb
            state={mapSessionState(state)}
            inputLevel={inputLevel}
            outputLevel={outputLevel}
            rippleSignals={rippleSignals}
            onRippleSignalsConsumed={onRippleSignalsConsumed}
          />
        </div>

        <div className="live-feedback">
          <div className="live-status" role="status" aria-live="polite">
            {labels.primary && (
              <span className="live-state-label">{labels.primary}</span>
            )}
            {labels.camera && (
              <span className="live-camera-label">{labels.camera}</span>
            )}
          </div>
          {assistantText && (
            <p className="live-assistant-caption" aria-live="polite">
              {assistantText}
            </p>
          )}
          <LiveCaption
            userText={userText}
            assistantText={assistantText}
            state={state}
          />
          {visibleErrorMessage && (
            <div className="live-error" role="alert">
              <X aria-hidden="true" />
              <span>{visibleErrorMessage}</span>
            </div>
          )}
        </div>
      </div>

      {hasOutput && (
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

      {(transcript.length > 0 || transcriptError) && (
        <aside className={`live-transcript ${transcriptExpanded ? 'is-expanded' : ''}`} aria-label="会议逐字稿">
          <button
            className="live-transcript-heading"
            type="button"
            aria-expanded={transcriptExpanded}
            onClick={() => setTranscriptExpanded((value) => !value)}
          >
            <span><i aria-hidden="true" />逐字稿</span>
            <small>{transcript.length} 段</small>
            {transcriptExpanded ? <ChevronDown aria-hidden="true" /> : <ChevronUp aria-hidden="true" />}
          </button>
          {transcriptError ? <p className="live-transcript-error">{transcriptError}</p> : null}
          <div className="live-transcript-list">
            {transcript.slice(transcriptExpanded ? 0 : -2).map((turn, index) => (
              <p key={`${turn.createdAt}-${index}`}>
                <strong>{turn.role === 'user' ? '我' : 'Ripple'}</strong>
                <span>{turn.text}</span>
              </p>
            ))}
          </div>
        </aside>
      )}

      <footer className="call-controls">
        <div className="call-control-item">
          <button
            className={`control-button ${muted ? 'is-active' : ''}`}
            type="button"
            aria-label={muted ? '取消静音' : '静音'}
            onClick={onToggleMute}
          >
            {muted ? <MicrophoneSlash aria-hidden="true" /> : <Microphone aria-hidden="true" />}
          </button>
          <span>{muted ? '取消静音' : '静音'}</span>
        </div>
        <div className="call-control-item">
          <button
            className="end-button"
            type="button"
            aria-label="结束通话"
            onClick={() => void onLeave()}
          >
            <Phone aria-hidden="true" />
          </button>
          <span>结束</span>
        </div>
        <div className="call-control-item">
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
            {videoMode ? <VideoCameraSlash aria-hidden="true" /> : <VideoCamera aria-hidden="true" />}
          </button>
          <span>{videoMode ? '关闭视频' : '切换到视频'}</span>
        </div>
      </footer>
    </section>
  )
}
