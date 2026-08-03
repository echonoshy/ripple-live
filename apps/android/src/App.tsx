import {
  ArrowLeft,
  CameraRotate,
  ChatCircleDots,
  ClockCounterClockwise,
  EnvelopeSimple,
  GearSix,
  HandPalm,
  LockKey,
  Microphone,
  MicrophoneSlash,
  PhoneDisconnect,
  SignOut,
  Ticket,
  VideoCamera,
  X,
} from '@phosphor-icons/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import './App.css'
import appIcon from '../src-tauri/icons/icon.png'
import {
  conversationMessages,
  conversations,
  currentUser,
  login,
  logout as logoutApi,
  register,
  type AuthUser,
  type ConversationMessage,
  type ConversationSummary,
} from './api'
import { LiveMedia } from './media/LiveMedia'
import {
  RealtimeSession,
  type RealtimeMode,
  type SessionState,
} from './realtime/RealtimeSession'

const DEFAULT_SERVER = '140.143.229.103:8700'

type Screen = 'home' | 'call' | 'settings' | 'history' | 'conversation'

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

function formatHistoryTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp * 1000))
}

function normalizeServerAddress(value: string) {
  return value
    .trim()
    .replace(/^https?:\/\//, '')
    .replace(/^wss?:\/\//, '')
    .replace(/\/+$/, '')
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('home')
  const [mode, setMode] = useState<RealtimeMode>('audio')
  const [serverDraft, setServerDraft] = useState(
    () => localStorage.getItem('ripple-agent-server') ?? DEFAULT_SERVER,
  )
  const [server, setServer] = useState(serverDraft)
  const [sessionState, setSessionState] = useState<SessionState>('idle')
  const [errorMessage, setErrorMessage] = useState('')
  const [assistantText, setAssistantText] = useState('')
  const [userText, setUserText] = useState('')
  const [toolStatus, setToolStatus] = useState('')
  const [muted, setMuted] = useState(false)
  const [elapsed, setElapsed] = useState(0)
  const [cameraFacing, setCameraFacing] = useState<'user' | 'environment'>(
    'environment',
  )
  const [accessToken, setAccessToken] = useState(
    () => localStorage.getItem('ripple-access-token') ?? '',
  )
  const [user, setUser] = useState<AuthUser | null>(null)
  const [authChecked, setAuthChecked] = useState(false)
  const [authMode, setAuthMode] = useState<'login' | 'register'>('login')
  const [authEmail, setAuthEmail] = useState('')
  const [authPassword, setAuthPassword] = useState('')
  const [invitationCode, setInvitationCode] = useState('')
  const [authError, setAuthError] = useState('')
  const [authBusy, setAuthBusy] = useState(false)
  const [historyItems, setHistoryItems] = useState<ConversationSummary[]>([])
  const [historyMessages, setHistoryMessages] = useState<ConversationMessage[]>([])
  const [historyBusy, setHistoryBusy] = useState(false)
  const [historyError, setHistoryError] = useState('')
  const [selectedConversation, setSelectedConversation] = useState<ConversationSummary | null>(null)

  const videoRef = useRef<HTMLVideoElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const sessionRef = useRef<RealtimeSession | null>(null)
  const mediaRef = useRef<LiveMedia | null>(null)
  const visualizerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let active = true
    if (!accessToken) {
      setAuthChecked(true)
      return
    }
    setAuthChecked(false)
    void currentUser(server, accessToken)
      .then((nextUser) => {
        if (active) setUser(nextUser)
      })
      .catch(() => {
        if (!active) return
        localStorage.removeItem('ripple-access-token')
        setAccessToken('')
        setUser(null)
      })
      .finally(() => {
        if (active) setAuthChecked(true)
      })
    return () => {
      active = false
    }
  }, [accessToken, server])

  useEffect(() => {
    if (screen !== 'history' || !accessToken) return
    let active = true
    setHistoryBusy(true)
    setHistoryError('')
    void conversations(server, accessToken)
      .then((items) => {
        if (active) setHistoryItems(items)
      })
      .catch((error: unknown) => {
        if (active) {
          setHistoryError(error instanceof Error ? error.message : '无法加载历史记录')
        }
      })
      .finally(() => {
        if (active) setHistoryBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, screen, server])

  useEffect(() => {
    if (screen !== 'conversation' || !accessToken || !selectedConversation) return
    let active = true
    setHistoryBusy(true)
    setHistoryError('')
    setHistoryMessages([])
    void conversationMessages(server, accessToken, selectedConversation.id)
      .then((items) => {
        if (active) setHistoryMessages(items)
      })
      .catch((error: unknown) => {
        if (active) {
          setHistoryError(error instanceof Error ? error.message : '无法加载聊天内容')
        }
      })
      .finally(() => {
        if (active) setHistoryBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, screen, selectedConversation, server])

  const isActive = [
    'connecting',
    'preparing',
    'listening',
    'thinking',
    'using_tool',
    'speaking',
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
      setToolStatus('')
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
        accessToken,
        mode: nextMode,
        onState: setSessionState,
        onError: (message) => {
          setErrorMessage(message)
          setSessionState('error')
        },
        onAssistantText: setAssistantText,
        onUserText: setUserText,
        onTool: setToolStatus,
        onAudio: (audio) => media.enqueueOutput(audio),
        onAudioDone: () => media.finishOutput(),
        onConversation: () => {},
        onReady: async () => {
          await media.start((audio, frame) => {
            void session.sendInput(audio, frame)
          }, () => {
            media.clearOutput()
            void session.speechStarted()
          }, () => {
            void session.commitInput()
          }, (level) => {
            visualizerRef.current?.style.setProperty('--audio-level', String(level))
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
    [accessToken, cameraFacing, server],
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
    const normalized = normalizeServerAddress(serverDraft)
    setServer(normalized || DEFAULT_SERVER)
    setServerDraft(normalized || DEFAULT_SERVER)
    localStorage.setItem('ripple-agent-server', normalized || DEFAULT_SERVER)
    setScreen('home')
  }

  const submitAuth = async () => {
    const normalized = normalizeServerAddress(serverDraft) || DEFAULT_SERVER
    setAuthBusy(true)
    setAuthError('')
    try {
      const session =
        authMode === 'login'
          ? await login(normalized, authEmail, authPassword)
          : await register(
              normalized,
              authEmail,
              authPassword,
              invitationCode,
            )
      setServer(normalized)
      setServerDraft(normalized)
      localStorage.setItem('ripple-agent-server', normalized)
      localStorage.setItem('ripple-access-token', session.access_token)
      setAccessToken(session.access_token)
      setUser(session.user)
      setAuthChecked(true)
      setAuthPassword('')
      setInvitationCode('')
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : '登录失败')
    } finally {
      setAuthBusy(false)
    }
  }

  const signOut = async () => {
    const token = accessToken
    localStorage.removeItem('ripple-access-token')
    setAccessToken('')
    setUser(null)
    setScreen('home')
    setHistoryItems([])
    setHistoryMessages([])
    if (token) await logoutApi(server, token).catch(() => {})
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

  if (!authChecked) {
    return (
      <main className="app-shell auth-shell">
        <section className="auth-screen auth-loading" aria-live="polite">
          <img src={appIcon} alt="" />
          <p>正在确认登录状态</p>
        </section>
      </main>
    )
  }

  if (!user) {
    return (
      <main className="app-shell auth-shell">
        <section className="auth-screen">
          <header className="auth-brand">
            <img src={appIcon} alt="" />
            <div>
              <strong>Ripple Live</strong>
              <span>登录你的私人实时 Agent</span>
            </div>
          </header>

          <div className="auth-intro">
            <p>{authMode === 'login' ? '欢迎回来' : '接受邀请'}</p>
            <h1>{authMode === 'login' ? '继续你的对话' : '创建私人账号'}</h1>
            <span>
              {authMode === 'login'
                ? '历史聊天只保存在你连接的 Ripple 服务中。'
                : '首次注册需要有效邀请码，邀请码受次数和有效期限制。'}
            </span>
          </div>

          <form
            className="auth-form"
            onSubmit={(event) => {
              event.preventDefault()
              void submitAuth()
            }}
          >
            <label htmlFor="auth-email">邮箱</label>
            <div className="field-control">
              <EnvelopeSimple aria-hidden="true" />
              <input
                id="auth-email"
                type="email"
                autoComplete="email"
                value={authEmail}
                onChange={(event) => setAuthEmail(event.target.value)}
                placeholder="you@example.com"
                required
              />
            </div>

            <label htmlFor="auth-password">密码</label>
            <div className="field-control">
              <LockKey aria-hidden="true" />
              <input
                id="auth-password"
                type="password"
                autoComplete={authMode === 'login' ? 'current-password' : 'new-password'}
                value={authPassword}
                onChange={(event) => setAuthPassword(event.target.value)}
                placeholder="至少 8 个字符"
                minLength={8}
                required
              />
            </div>

            {authMode === 'register' && (
              <>
                <label htmlFor="invitation-code">邀请码</label>
                <div className="field-control">
                  <Ticket aria-hidden="true" />
                  <input
                    id="invitation-code"
                    value={invitationCode}
                    onChange={(event) => setInvitationCode(event.target.value)}
                    placeholder="输入邀请码"
                    required
                  />
                </div>
              </>
            )}

            <label htmlFor="auth-server">服务地址</label>
            <input
              className="server-field"
              id="auth-server"
              value={serverDraft}
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              onChange={(event) => setServerDraft(event.target.value)}
              required
            />

            {authError && <p className="form-error">{authError}</p>}
            <button className="primary-button" type="submit" disabled={authBusy}>
              {authBusy
                ? '正在连接'
                : authMode === 'login'
                  ? '登录'
                  : '创建账号'}
            </button>
          </form>

          <button
            className="auth-switch"
            type="button"
            onClick={() => {
              setAuthMode(authMode === 'login' ? 'register' : 'login')
              setAuthError('')
            }}
          >
            {authMode === 'login' ? '有邀请码？创建账号' : '已有账号？返回登录'}
          </button>
        </section>
      </main>
    )
  }

  return (
    <main className="app-shell">
      {screen === 'home' && (
        <section className="home-screen">
          <header className="home-header">
            <div className="brand-lockup">
              <img src={appIcon} alt="" />
              <div>
                <strong>Ripple Live</strong>
                <span>私人实时 Agent</span>
              </div>
            </div>
            <div className="header-actions">
              <button
                className="icon-button"
                type="button"
                aria-label="聊天历史"
                onClick={() => setScreen('history')}
              >
                <ClockCounterClockwise />
              </button>
              <button
                className="icon-button"
                type="button"
                aria-label="打开设置"
                onClick={() => setScreen('settings')}
              >
                <GearSix />
              </button>
            </div>
          </header>

          <div className="ready-state">
            <div className="ready-mark" aria-hidden="true">
              <img src={appIcon} alt="" />
            </div>
            <p><span aria-hidden="true" /> 准备就绪</p>
            <h1>今天想聊什么？</h1>
            <span>我可以听、看，也可以帮你使用工具。</span>
          </div>

          <div className="launch-actions">
            <button
              className="launch-button primary-launch"
              type="button"
              onClick={() => openCall('audio')}
            >
              <Microphone weight="fill" />
              <span>
                <strong>开始语音通话</strong>
                <small>语音模式</small>
              </span>
            </button>
            <button
              className="launch-button secondary-launch"
              type="button"
              onClick={() => openCall('video')}
            >
              <VideoCamera weight="fill" />
              <span>
                <strong>打开视频通话</strong>
                <small>视频模式</small>
              </span>
            </button>
          </div>
        </section>
      )}

      {screen === 'history' && (
        <section className="history-screen">
          <header className="screen-header">
            <button
              className="icon-button"
              type="button"
              aria-label="返回"
              onClick={() => setScreen('home')}
            >
              <ArrowLeft />
            </button>
            <h1>聊天历史</h1>
            <span className="header-spacer" />
          </header>

          <div className="history-heading">
            <p>{user.email}</p>
            <h2>最近聊过的内容</h2>
          </div>

          {historyBusy && (
            <div className="history-skeleton" aria-label="正在加载">
              <span />
              <span />
              <span />
            </div>
          )}
          {historyError && <div className="history-error">{historyError}</div>}
          {!historyBusy && !historyError && historyItems.length === 0 && (
            <div className="history-empty">
              <ChatCircleDots />
              <h2>还没有聊天记录</h2>
              <p>完成第一次语音或视频对话后，文本内容会出现在这里。</p>
              <button type="button" onClick={() => openCall('audio')}>
                开始语音通话
              </button>
            </div>
          )}
          {!historyBusy && historyItems.length > 0 && (
            <div className="history-list">
              {historyItems.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => {
                    setSelectedConversation(item)
                    setScreen('conversation')
                  }}
                >
                  <div>
                    <strong>{item.title || '未命名对话'}</strong>
                    <time>{formatHistoryTime(item.updated_at)}</time>
                  </div>
                  <p>{item.preview || '这次对话还没有文本内容'}</p>
                </button>
              ))}
            </div>
          )}
        </section>
      )}

      {screen === 'conversation' && selectedConversation && (
        <section className="history-screen conversation-history-screen">
          <header className="screen-header">
            <button
              className="icon-button"
              type="button"
              aria-label="返回聊天历史"
              onClick={() => setScreen('history')}
            >
              <ArrowLeft />
            </button>
            <h1>聊天内容</h1>
            <span className="header-spacer" />
          </header>

          <div className="conversation-title">
            <h2>{selectedConversation.title || '未命名对话'}</h2>
            <time>{formatHistoryTime(selectedConversation.updated_at)}</time>
          </div>

          {historyBusy && (
            <div className="message-skeleton" aria-label="正在加载聊天内容">
              <span />
              <span />
              <span />
            </div>
          )}
          {historyError && <div className="history-error">{historyError}</div>}
          {!historyBusy && !historyError && (
            <div className="message-history">
              {historyMessages.map((message) => (
                <article
                  key={message.id}
                  className={message.role === 'user' ? 'is-user' : 'is-assistant'}
                >
                  <div>
                    <strong>{message.role === 'user' ? '你' : 'Ripple'}</strong>
                    <time>{formatHistoryTime(message.created_at)}</time>
                  </div>
                  <p>{message.content}</p>
                </article>
              ))}
            </div>
          )}
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
              placeholder="140.143.229.103:8700"
            />
            <p>使用明文 WebSocket 连接。只需填写 IP 和端口。</p>
            <button className="primary-button" type="button" onClick={saveSettings}>
              保存
            </button>
          </div>
          <div className="account-panel">
            <div>
              <span>当前账号</span>
              <strong>{user.email}</strong>
            </div>
            <button type="button" onClick={() => void signOut()}>
              <SignOut />
              退出登录
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
            <span className="call-mode">
              {mode === 'video' ? '视频通话' : '语音通话'}
            </span>
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

          <div className={`conversation ${statusClass}`}>
            {mode === 'audio' && (
              <div
                ref={visualizerRef}
                className={`voice-visualizer ${statusClass}`}
                aria-hidden="true"
              >
                {[0.45, 0.7, 1, 0.62, 0.88, 0.56, 0.38].map((scale, index) => (
                  <span
                    key={index}
                    style={{ height: `${24 + scale * 88}px` }}
                  />
                ))}
              </div>
            )}

            <div className="transcript" aria-live="polite">
              {userText && (
                <div className="utterance user-utterance">
                  <span>你</span>
                  <p>{userText}</p>
                </div>
              )}
              <div className="utterance assistant-utterance">
                <span>Ripple</span>
                <p>
                  {assistantText ||
                    (sessionState === 'listening'
                      ? '我在听'
                      : sessionState === 'thinking'
                        ? '正在理解你的问题'
                        : sessionState === 'using_tool'
                          ? toolStatus || '正在使用工具'
                      : sessionState === 'speaking'
                        ? '正在回答'
                        : '正在建立实时连接')}
                </p>
              </div>
              {errorMessage && (
                <div className="error-message">
                  <X weight="bold" />
                  <span>{errorMessage}</span>
                </div>
              )}
            </div>
          </div>

          <footer className="call-controls">
            <div className="control-item">
              <button
                className={`control-button ${muted ? 'is-active' : ''}`}
                type="button"
                aria-label={muted ? '取消静音' : '静音'}
                onClick={toggleMute}
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
                onClick={() => void leaveCall()}
              >
                <PhoneDisconnect weight="fill" />
              </button>
              <span>结束</span>
            </div>
            <div className="control-item">
              <button
                className="control-button"
                type="button"
                aria-label="打断回答"
                onClick={() => {
                  if (sessionRef.current?.forceListen()) {
                    mediaRef.current?.clearOutput()
                  }
                }}
              >
                <HandPalm weight="fill" />
              </button>
              <span>打断</span>
            </div>
          </footer>
        </section>
      )}
    </main>
  )
}
