import {
  ArrowLeft,
  Camera,
  CameraRotate,
  GearSix,
  Microphone,
  MicrophoneSlash,
  PhoneDisconnect,
  SpeakerHigh,
  VideoCamera,
  Waveform,
  X,
} from '@phosphor-icons/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import './App.css'
import { LiveMedia } from './media/LiveMedia'
import {
  RealtimeSession,
  type RealtimeMode,
  type SessionState,
} from './realtime/RealtimeSession'

const DEFAULT_SERVER = '140.143.229.103:8600'

type Screen = 'home' | 'call' | 'settings'

const stateLabels: Record<SessionState, string> = {
  idle: '准备就绪',
  connecting: '正在连接',
  queued: '正在排队',
  preparing: '正在准备模型',
  listening: '正在聆听',
  speaking: '正在回答',
  paused: '已暂停',
  ended: '通话已结束',
  error: '连接异常',
}

function formatDuration(seconds: number) {
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return `${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}`
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('home')
  const [mode, setMode] = useState<RealtimeMode>('audio')
  const [serverDraft, setServerDraft] = useState(
    () => localStorage.getItem('minicpm-server') ?? DEFAULT_SERVER,
  )
  const [server, setServer] = useState(serverDraft)
  const [sessionState, setSessionState] = useState<SessionState>('idle')
  const [errorMessage, setErrorMessage] = useState('')
  const [assistantText, setAssistantText] = useState('')
  const [userText, setUserText] = useState('')
  const [muted, setMuted] = useState(false)
  const [elapsed, setElapsed] = useState(0)
  const [cameraFacing, setCameraFacing] = useState<'user' | 'environment'>(
    'environment',
  )

  const videoRef = useRef<HTMLVideoElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const sessionRef = useRef<RealtimeSession | null>(null)
  const mediaRef = useRef<LiveMedia | null>(null)

  const isActive = [
    'connecting',
    'queued',
    'preparing',
    'listening',
    'speaking',
    'paused',
  ].includes(sessionState)

  useEffect(() => {
    if (!isActive) return
    const timer = window.setInterval(() => setElapsed((value) => value + 1), 1000)
    return () => window.clearInterval(timer)
  }, [isActive])

  useEffect(() => {
    if (sessionState !== 'ended' && sessionState !== 'error') return
    mediaRef.current?.stop()
    mediaRef.current = null
  }, [sessionState])

  const stopCall = useCallback(async () => {
    mediaRef.current?.stop()
    mediaRef.current = null
    await sessionRef.current?.close()
    sessionRef.current = null
    setSessionState('ended')
  }, [])

  useEffect(() => {
    return () => {
      mediaRef.current?.stop()
      void sessionRef.current?.close()
    }
  }, [])

  const startCall = useCallback(
    async (nextMode: RealtimeMode) => {
      if (!videoRef.current || !canvasRef.current) return

      setMode(nextMode)
      setErrorMessage('')
      setAssistantText('')
      setUserText('')
      setElapsed(0)
      setMuted(false)
      setSessionState('connecting')

      const media = new LiveMedia({
        video: videoRef.current,
        canvas: canvasRef.current,
        withVideo: nextMode === 'video',
        facingMode: cameraFacing,
      })
      const session = new RealtimeSession({
        server,
        mode: nextMode,
        onState: setSessionState,
        onError: (message) => {
          setErrorMessage(message)
          setSessionState('error')
        },
        onAssistantText: setAssistantText,
        onUserText: setUserText,
        onAudio: (audio) => media.enqueueOutput(audio),
        onReady: async () => {
          await media.start((audio, frame) => {
            void session.sendInput(audio, frame)
          })
        },
      })

      mediaRef.current = media
      sessionRef.current = session

      try {
        await session.connect()
      } catch (error) {
        media.stop()
        const message =
          error instanceof Error ? error.message : '无法连接实时服务'
        setErrorMessage(message)
        setSessionState('error')
      }
    },
    [cameraFacing, server],
  )

  useEffect(() => {
    if (screen !== 'call' || sessionRef.current) return
    const frame = window.requestAnimationFrame(() => void startCall(mode))
    return () => window.cancelAnimationFrame(frame)
  }, [mode, screen, startCall])

  const openCall = (nextMode: RealtimeMode) => {
    setMode(nextMode)
    setSessionState('idle')
    setScreen('call')
  }

  const leaveCall = async () => {
    await stopCall()
    setScreen('home')
    setSessionState('idle')
  }

  const toggleMute = () => {
    const next = !muted
    setMuted(next)
    mediaRef.current?.setMuted(next)
  }

  const flipCamera = async () => {
    const next = cameraFacing === 'user' ? 'environment' : 'user'
    setCameraFacing(next)
    await mediaRef.current?.setFacingMode(next)
  }

  const saveSettings = () => {
    const normalized = serverDraft
      .trim()
      .replace(/^wss?:\/\//, '')
      .replace(/\/+$/, '')
    setServer(normalized || DEFAULT_SERVER)
    setServerDraft(normalized || DEFAULT_SERVER)
    localStorage.setItem('minicpm-server', normalized || DEFAULT_SERVER)
    setScreen('home')
  }

  const statusClass = useMemo(
    () =>
      sessionState === 'error'
        ? 'is-error'
        : sessionState === 'speaking'
          ? 'is-speaking'
          : isActive
            ? 'is-live'
            : '',
    [isActive, sessionState],
  )

  return (
    <main className="app-shell">
      {screen === 'home' && (
        <section className="home-screen">
          <header className="home-header">
            <div className="brand-mark" aria-hidden="true">
              <Waveform weight="bold" />
            </div>
            <button
              className="icon-button"
              type="button"
              aria-label="打开设置"
              onClick={() => setScreen('settings')}
            >
              <GearSix />
            </button>
          </header>

          <div className="home-copy">
            <p className="product-name">Ripple Live</p>
            <h1>现在，就开始对话</h1>
            <p>直接说话，或者打开摄像头。模型会边听、边看、边回答。</p>
          </div>

          <div className="mode-actions">
            <button
              className="mode-button audio-mode"
              type="button"
              onClick={() => openCall('audio')}
            >
              <span className="mode-icon">
                <Microphone weight="fill" />
              </span>
              <span>
                <strong>语音通话</strong>
                <small>端到端实时语音</small>
              </span>
            </button>
            <button
              className="mode-button video-mode"
              type="button"
              onClick={() => openCall('video')}
            >
              <span className="mode-icon">
                <VideoCamera weight="fill" />
              </span>
              <span>
                <strong>视频通话</strong>
                <small>让模型听见并看见</small>
              </span>
            </button>
          </div>

          <div className="server-line">
            <span className="server-indicator" aria-hidden="true" />
            <span>{server}</span>
          </div>
        </section>
      )}

      {screen === 'settings' && (
        <section className="settings-screen">
          <header className="screen-header">
            <button
              className="icon-button"
              type="button"
              aria-label="返回"
              onClick={() => setScreen('home')}
            >
              <ArrowLeft />
            </button>
            <h1>连接设置</h1>
            <span className="header-spacer" />
          </header>

          <div className="settings-form">
            <label htmlFor="server">服务地址</label>
            <input
              id="server"
              value={serverDraft}
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              onChange={(event) => setServerDraft(event.target.value)}
              placeholder="140.143.229.103:8600"
            />
            <p>使用明文 WebSocket 连接。只需填写 IP 和端口。</p>
            <button className="primary-button" type="button" onClick={saveSettings}>
              保存
            </button>
          </div>
        </section>
      )}

      {screen === 'call' && (
        <section className={`call-screen ${mode === 'video' ? 'has-video' : ''}`}>
          <video
            ref={videoRef}
            className="camera-preview"
            autoPlay
            muted
            playsInline
          />
          <canvas ref={canvasRef} hidden />
          <div className="camera-scrim" />

          <header className="call-header">
            <button
              className="icon-button call-icon"
              type="button"
              aria-label="退出通话"
              onClick={() => void leaveCall()}
            >
              <ArrowLeft />
            </button>
            <div className={`call-status ${statusClass}`}>
              <span aria-hidden="true" />
              <strong>{stateLabels[sessionState]}</strong>
              <small>{formatDuration(elapsed)}</small>
            </div>
            {mode === 'video' ? (
              <button
                className="icon-button call-icon"
                type="button"
                aria-label="切换摄像头"
                onClick={() => void flipCamera()}
              >
                <CameraRotate />
              </button>
            ) : (
              <span className="header-spacer" />
            )}
          </header>

          <div className="conversation">
            {mode === 'audio' && (
              <div className={`voice-orb ${statusClass}`}>
                <div className="orb-core">
                  {sessionState === 'speaking' ? (
                    <SpeakerHigh weight="fill" />
                  ) : (
                    <Waveform weight="bold" />
                  )}
                </div>
                <span className="orb-ring ring-one" />
                <span className="orb-ring ring-two" />
              </div>
            )}

            <div className="transcript" aria-live="polite">
              {userText && <p className="user-caption">{userText}</p>}
              <p className="assistant-caption">
                {assistantText ||
                  (sessionState === 'listening'
                    ? '我在听'
                    : sessionState === 'speaking'
                      ? '正在组织回答'
                      : '正在建立实时连接')}
              </p>
              {errorMessage && (
                <div className="error-message">
                  <X weight="bold" />
                  <span>{errorMessage}</span>
                </div>
              )}
            </div>
          </div>

          <footer className="call-controls">
            <button
              className={`control-button ${muted ? 'is-active' : ''}`}
              type="button"
              aria-label={muted ? '取消静音' : '静音'}
              onClick={toggleMute}
            >
              {muted ? <MicrophoneSlash /> : <Microphone />}
            </button>
            <button
              className="end-button"
              type="button"
              aria-label="结束通话"
              onClick={() => void leaveCall()}
            >
              <PhoneDisconnect weight="fill" />
            </button>
            <button
              className="control-button"
              type="button"
              aria-label="强制模型聆听"
              onClick={() => sessionRef.current?.forceListen()}
            >
              {mode === 'video' ? <Camera /> : <Waveform />}
            </button>
          </footer>
        </section>
      )}
    </main>
  )
}
