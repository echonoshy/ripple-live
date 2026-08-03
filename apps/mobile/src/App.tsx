import {
  ArrowLeft,
  CameraRotate,
  ChatCircleDots,
  CheckCircle,
  ClockCounterClockwise,
  Circle,
  EnvelopeSimple,
  GearSix,
  HandPalm,
  ImagesSquare,
  LockKey,
  ListChecks,
  Microphone,
  MicrophoneSlash,
  PhoneDisconnect,
  PushPin,
  SignOut,
  NotePencil,
  Trash,
  Ticket,
  VideoCamera,
  X,
} from '@phosphor-icons/react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import './App.css'
import appIcon from '../src-tauri/icons/icon.png'
import {
  assetBlob,
  batchConversations,
  batchMemories,
  conversationMessages,
  conversationMutation,
  conversations,
  currentUser,
  login,
  logout as logoutApi,
  memories,
  memoryMutation,
  register,
  updateMemory,
  todos,
  updateTodo,
  type AuthUser,
  type ConversationMessage,
  type ConversationSummary,
  type MemoryArtifact,
  type TodoItem,
  type VisualMemory,
} from './api'
import { LibraryActions } from './components/LibraryActions'
import { LibrarySection } from './components/LibrarySection'
import { LibraryToolbar } from './components/LibraryToolbar'
import { MarkdownContent } from './components/MarkdownContent'
import {
  groupLibraryItems,
  libraryOptionsForView,
  matchesLibraryQuery,
  type LibraryAction,
  type LibraryItem,
  type LibraryView,
} from './library'
import { LiveMedia } from './media/LiveMedia'
import { notifyDueTodos } from './reminders'
import {
  RealtimeSession,
  type ResponseArtifact,
  type RealtimeMode,
  type SessionState,
} from './realtime/RealtimeSession'

const DEFAULT_SERVER = '140.143.229.103:8700'

type Screen =
  | 'home'
  | 'call'
  | 'settings'
  | 'history'
  | 'conversation'
  | 'memories'
  | 'todos'

function AuthenticatedImage({
  server,
  token,
  artifact,
  className,
}: {
  server: string
  token: string
  artifact: MemoryArtifact | ResponseArtifact
  className?: string
}) {
  const [source, setSource] = useState('')

  useEffect(() => {
    let active = true
    let objectUrl = ''
    void assetBlob(server, token, artifact.content_url)
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
  }, [artifact.content_url, server, token])

  if (!source) return <div className={`memory-image-placeholder ${className ?? ''}`} />
  return (
    <img
      className={className}
      src={source}
      alt={artifact.caption || '保存的记忆画面'}
    />
  )
}

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

export default function App() {
  const [screen, setScreen] = useState<Screen>('home')
  const [mode, setMode] = useState<RealtimeMode>('audio')
  const server = DEFAULT_SERVER
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
  const [historyQuery, setHistoryQuery] = useState('')
  const [debouncedHistoryQuery, setDebouncedHistoryQuery] = useState('')
  const [historyScope, setHistoryScope] = useState<LibraryView>('all')
  const [historySelection, setHistorySelection] = useState<Set<string>>(new Set())
  const [selectedConversation, setSelectedConversation] = useState<ConversationSummary | null>(null)
  const [memoryItems, setMemoryItems] = useState<VisualMemory[]>([])
  const [memoryBusy, setMemoryBusy] = useState(false)
  const [memoryError, setMemoryError] = useState('')
  const [memoryQuery, setMemoryQuery] = useState('')
  const [debouncedMemoryQuery, setDebouncedMemoryQuery] = useState('')
  const [memoryScope, setMemoryScope] = useState<LibraryView>('all')
  const [memorySelection, setMemorySelection] = useState<Set<string>>(new Set())
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null)
  const [editingMemoryId, setEditingMemoryId] = useState<string | null>(null)
  const [memoryDraft, setMemoryDraft] = useState('')
  const [todoItems, setTodoItems] = useState<TodoItem[]>([])
  const [todoBusy, setTodoBusy] = useState(false)
  const [todoError, setTodoError] = useState('')
  const [revealedItem, setRevealedItem] = useState<string | null>(null)
  const [deleteRequest, setDeleteRequest] = useState<{
    kind: 'history' | 'memory'
    ids: string[]
  } | null>(null)
  const [liveArtifacts, setLiveArtifacts] = useState<ResponseArtifact[]>([])

  const videoRef = useRef<HTMLVideoElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const sessionRef = useRef<RealtimeSession | null>(null)
  const mediaRef = useRef<LiveMedia | null>(null)
  const visualizerRef = useRef<HTMLDivElement>(null)
  const longPressTimerRef = useRef<number | null>(null)
  const pointerStartRef = useRef<{ id: string; x: number; y: number } | null>(null)
  const suppressClickRef = useRef(false)

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedHistoryQuery(historyQuery), 250)
    return () => window.clearTimeout(timer)
  }, [historyQuery])

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedMemoryQuery(memoryQuery), 250)
    return () => window.clearTimeout(timer)
  }, [memoryQuery])

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
    void conversations(
      server,
      accessToken,
      libraryOptionsForView(historyScope, debouncedHistoryQuery, 100),
    )
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
  }, [accessToken, debouncedHistoryQuery, historyScope, screen, server])

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

  useEffect(() => {
    if (screen !== 'memories' || !accessToken) return
    let active = true
    setMemoryBusy(true)
    setMemoryError('')
    void memories(
      server,
      accessToken,
      libraryOptionsForView(memoryScope, debouncedMemoryQuery, 100),
    )
      .then((items) => {
        if (active) setMemoryItems(items)
      })
      .catch((error: unknown) => {
        if (active) {
          setMemoryError(error instanceof Error ? error.message : '无法加载视觉记忆')
        }
      })
      .finally(() => {
        if (active) setMemoryBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, debouncedMemoryQuery, memoryScope, screen, server])

  useEffect(() => {
    if (screen !== 'todos' || !accessToken) return
    let active = true
    setTodoBusy(true)
    setTodoError('')
    void todos(server, accessToken)
      .then((items) => {
        if (active) setTodoItems(items)
      })
      .catch((error: unknown) => {
        if (active) setTodoError(error instanceof Error ? error.message : '无法加载待办')
      })
      .finally(() => {
        if (active) setTodoBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, screen, server])

  useEffect(() => {
    if (!accessToken) return
    const checkReminders = () => {
      void todos(server, accessToken)
        .then(notifyDueTodos)
        .catch(() => {})
    }
    checkReminders()
    const timer = window.setInterval(checkReminders, 60_000)
    return () => window.clearInterval(timer)
  }, [accessToken, server])

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
      setLiveArtifacts([])
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
        onUserText: (text) => {
          setUserText(text)
          setLiveArtifacts([])
        },
        onTool: setToolStatus,
        onAudio: (audio) => media.enqueueOutput(audio),
        onAudioDone: () => media.finishOutput(),
        onArtifact: (artifact) => {
          setLiveArtifacts((items) =>
            items.some((item) => item.id === artifact.id)
              ? items
              : [...items, artifact],
          )
        },
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

  const submitAuth = async () => {
    setAuthBusy(true)
    setAuthError('')
    try {
      const session =
        authMode === 'login'
          ? await login(server, authEmail, authPassword)
          : await register(
              server,
              authEmail,
              authPassword,
              invitationCode,
            )
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
    setMemoryItems([])
    setTodoItems([])
    if (token) await logoutApi(server, token).catch(() => {})
  }

  const saveMemoryEdit = async (memoryId: string) => {
    const note = memoryDraft.trim()
    if (!note) return
    setMemoryError('')
    try {
      const updated = await updateMemory(server, accessToken, memoryId, note)
      setMemoryItems((items) =>
        items.map((item) => (item.id === memoryId ? updated : item)),
      )
      setEditingMemoryId(null)
      setMemoryDraft('')
    } catch (error) {
      setMemoryError(error instanceof Error ? error.message : '无法修改记忆')
    }
  }

  const setTodoCompleted = async (todo: TodoItem) => {
    const previous = todoItems
    setTodoItems((items) => items.filter((item) => item.id !== todo.id))
    try {
      await updateTodo(server, accessToken, todo.id, true)
    } catch (error) {
      setTodoItems(previous)
      setTodoError(error instanceof Error ? error.message : '无法更新待办')
    }
  }

  const historyLibraryItems = useMemo(
    () =>
      historyItems
        .map((item): LibraryItem => ({
          id: item.id,
          title: item.title || '未命名对话',
          searchableText: `${item.title} ${item.preview}`,
          timestamp: item.updated_at,
          isPinned: item.is_pinned,
          archivedAt: item.archived_at,
        }))
        .filter((item) => matchesLibraryQuery(item, historyQuery)),
    [historyItems, historyQuery],
  )
  const memoryLibraryItems = useMemo(
    () =>
      memoryItems
        .map((item): LibraryItem => ({
          id: item.id,
          title: item.user_note || '未命名记忆',
          searchableText: `${item.user_note} ${item.visual_summary}`,
          timestamp: item.captured_at ?? item.created_at,
          isPinned: item.is_pinned,
          archivedAt: item.archived_at,
        }))
        .filter((item) => matchesLibraryQuery(item, memoryQuery)),
    [memoryItems, memoryQuery],
  )
  const historyGroups = useMemo(
    () => groupLibraryItems(historyLibraryItems, new Date(), historyScope),
    [historyLibraryItems, historyScope],
  )
  const memoryGroups = useMemo(
    () => groupLibraryItems(memoryLibraryItems, new Date(), memoryScope),
    [memoryLibraryItems, memoryScope],
  )
  const selectedMemory = selectedMemoryId
    ? memoryItems.find((item) => item.id === selectedMemoryId) ?? null
    : null

  const toggleSelection = (
    setter: React.Dispatch<React.SetStateAction<Set<string>>>,
    id: string,
  ) => {
    setter((selected) => {
      const next = new Set(selected)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const cancelLongPress = () => {
    if (longPressTimerRef.current !== null) {
      window.clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }

  const beginLibraryGesture = (
    event: React.PointerEvent<HTMLElement>,
    id: string,
    select: () => void,
  ) => {
    if ((event.target as HTMLElement).closest('.library-item-actions, input, textarea')) return
    cancelLongPress()
    pointerStartRef.current = { id, x: event.clientX, y: event.clientY }
    suppressClickRef.current = false
    longPressTimerRef.current = window.setTimeout(() => {
      suppressClickRef.current = true
      select()
      longPressTimerRef.current = null
    }, 500)
  }

  const moveLibraryGesture = (event: React.PointerEvent<HTMLElement>) => {
    const start = pointerStartRef.current
    if (!start) return
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) > 10) {
      cancelLongPress()
    }
  }

  const endLibraryGesture = (event: React.PointerEvent<HTMLElement>) => {
    const start = pointerStartRef.current
    cancelLongPress()
    pointerStartRef.current = null
    if (!start) return
    const horizontalDistance = event.clientX - start.x
    if (horizontalDistance < -44) {
      suppressClickRef.current = true
      setRevealedItem(start.id)
    } else if (horizontalDistance > 32) {
      suppressClickRef.current = true
      setRevealedItem(null)
    }
  }

  const optimisticHistoryAction = async (ids: string[], action: LibraryAction) => {
    if (action === 'delete') {
      setDeleteRequest({ kind: 'history', ids })
      return
    }
    const previous = historyItems
    const selected = new Set(ids)
    const archivedAt = Date.now() / 1000
    setHistoryItems((items) =>
      items.map((item) => {
        if (!selected.has(item.id)) return item
        if (action === 'pin') return { ...item, is_pinned: true }
        if (action === 'unpin') return { ...item, is_pinned: false }
        return { ...item, archived_at: action === 'archive' ? archivedAt : null }
      }),
    )
    setHistoryError('')
    try {
      if (ids.length === 1) {
        await conversationMutation(server, accessToken, ids[0], action)
      } else {
        await batchConversations(server, accessToken, ids, action)
      }
      setHistorySelection(new Set())
      setRevealedItem(null)
    } catch (error) {
      setHistoryItems(previous)
      setHistoryError(error instanceof Error ? error.message : '操作失败，请重试')
    }
  }

  const optimisticMemoryAction = async (ids: string[], action: LibraryAction) => {
    if (action === 'delete') {
      setDeleteRequest({ kind: 'memory', ids })
      return
    }
    const previous = memoryItems
    const selected = new Set(ids)
    const archivedAt = Date.now() / 1000
    setMemoryItems((items) =>
      items.map((item) => {
        if (!selected.has(item.id)) return item
        if (action === 'pin') return { ...item, is_pinned: true }
        if (action === 'unpin') return { ...item, is_pinned: false }
        return { ...item, archived_at: action === 'archive' ? archivedAt : null }
      }),
    )
    setMemoryError('')
    try {
      if (ids.length === 1) {
        await memoryMutation(server, accessToken, ids[0], action)
      } else {
        await batchMemories(server, accessToken, ids, action)
      }
      setMemorySelection(new Set())
      setRevealedItem(null)
    } catch (error) {
      setMemoryItems(previous)
      setMemoryError(error instanceof Error ? error.message : '操作失败，请重试')
    }
  }

  const confirmDelete = async () => {
    if (!deleteRequest) return
    const { kind, ids } = deleteRequest
    const setError = kind === 'history' ? setHistoryError : setMemoryError
    setError('')
    try {
      if (kind === 'history') {
        if (ids.length === 1) {
          await conversationMutation(server, accessToken, ids[0], 'delete')
        } else {
          await batchConversations(server, accessToken, ids, 'delete')
        }
        setHistoryItems((items) => items.filter((item) => !ids.includes(item.id)))
        setHistorySelection(new Set())
      } else {
        if (ids.length === 1) {
          await memoryMutation(server, accessToken, ids[0], 'delete')
        } else {
          await batchMemories(server, accessToken, ids, 'delete')
        }
        setMemoryItems((items) => items.filter((item) => !ids.includes(item.id)))
        setMemorySelection(new Set())
        if (selectedMemoryId && ids.includes(selectedMemoryId)) setSelectedMemoryId(null)
      }
      setRevealedItem(null)
      setDeleteRequest(null)
    } catch (error) {
      setError(error instanceof Error ? error.message : '删除失败，请重试')
      setDeleteRequest(null)
    }
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
                aria-label="待办"
                onClick={() => setScreen('todos')}
              >
                <ListChecks />
              </button>
              <button
                className="icon-button"
                type="button"
                aria-label="视觉记忆"
                onClick={() => setScreen('memories')}
              >
                <ImagesSquare />
              </button>
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
            <h1>打开镜头，开始聊聊</h1>
            <span>让我看见现场，实时听懂并回应你。</span>
          </div>

          <div className="launch-actions">
            <button
              className="launch-button call-entry is-video"
              type="button"
              onClick={() => openCall('video')}
            >
              <span className="call-entry-icon" aria-hidden="true">
                <VideoCamera weight="fill" />
              </span>
              <span className="call-entry-copy">
                <strong>开始视频通话</strong>
                <small>让我看见现场</small>
              </span>
            </button>
            <button
              className="launch-button call-entry is-voice"
              type="button"
              aria-label="开始语音通话"
              onClick={() => openCall('audio')}
            >
              <span className="call-entry-icon" aria-hidden="true">
                <Microphone weight="fill" />
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

          <div className="library-region" aria-label="搜索聊天历史">
            <LibraryToolbar
              kind="聊天历史"
              query={historyQuery}
              scope={historyScope}
              selectionCount={historySelection.size}
              onQueryChange={setHistoryQuery}
              onScopeChange={(scope) => {
                setHistoryScope(scope)
                setHistorySelection(new Set())
                setRevealedItem(null)
              }}
              onBatchAction={(action) =>
                void optimisticHistoryAction([...historySelection], action)
              }
              onCancelSelection={() => setHistorySelection(new Set())}
            />
          </div>

          {historyBusy && (
            <div className="history-skeleton" aria-label="正在加载">
              <span />
              <span />
              <span />
            </div>
          )}
          {historyError && <div className="history-error">{historyError}</div>}
          {!historyBusy && !historyError && historyGroups.length === 0 && (
            <div className="history-empty">
              <ChatCircleDots />
              <h2>{historyQuery ? '没有找到相关对话' : historyScope === 'archived' ? '还没有归档对话' : historyScope === 'pinned' ? '还没有标记对话' : '还没有聊天记录'}</h2>
              <p>
                {historyScope === 'archived'
                  ? '已归档的对话会保留，但不会出现在最近记录中。'
                  : historyQuery
                    ? '试试更短的关键词，或清除搜索条件。'
                    : '完成第一次语音或视频对话后，文本内容会出现在这里。'}
              </p>
              {historyQuery ? (
                <button type="button" onClick={() => setHistoryQuery('')}>清除搜索</button>
              ) : historyScope === 'all' ? (
                <button type="button" onClick={() => openCall('audio')}>开始语音通话</button>
              ) : null}
            </div>
          )}
          {!historyBusy && historyGroups.length > 0 && (
            <div className="library-groups history-list">
              {historyGroups.map((group) => (
                <LibrarySection key={group.label} label={group.label} count={group.items.length}>
                  <div className="library-section-items">
                    {group.items.map((libraryItem) => {
                      const item = historyItems.find((candidate) => candidate.id === libraryItem.id)
                      if (!item) return null
                      const selected = historySelection.has(item.id)
                      return (
                        <article
                          key={item.id}
                          className={`library-swipe-shell ${revealedItem === item.id ? 'is-revealed' : ''}`}
                          onPointerDown={(event) =>
                            beginLibraryGesture(event, item.id, () =>
                              toggleSelection(setHistorySelection, item.id),
                            )
                          }
                          onPointerMove={moveLibraryGesture}
                          onPointerUp={endLibraryGesture}
                          onPointerCancel={() => {
                            cancelLongPress()
                            pointerStartRef.current = null
                          }}
                        >
                          <button
                            className="history-row library-item-surface"
                            type="button"
                            aria-pressed={selected}
                            onClick={() => {
                              if (suppressClickRef.current) {
                                suppressClickRef.current = false
                                return
                              }
                              if (historySelection.size > 0) {
                                toggleSelection(setHistorySelection, item.id)
                                return
                              }
                              setSelectedConversation(item)
                              setScreen('conversation')
                            }}
                          >
                            {historySelection.size > 0 && (
                              <span className={`selection-check ${selected ? 'is-selected' : ''}`} aria-hidden="true" />
                            )}
                            <span className="library-row-copy">
                              <span className="library-row-title">
                                <strong>{item.title || '未命名对话'}</strong>
                                {item.is_pinned && <PushPin weight="fill" aria-label="已标记" />}
                                <time>{formatHistoryTime(item.updated_at)}</time>
                              </span>
                              <span className="library-row-preview">{item.preview || '这次对话还没有文本内容'}</span>
                            </span>
                          </button>
                          <LibraryActions
                            pinned={item.is_pinned}
                            archived={item.archived_at !== null}
                            onAction={(action) => void optimisticHistoryAction([item.id], action)}
                          />
                        </article>
                      )
                    })}
                  </div>
                </LibrarySection>
              ))}
            </div>
          )}
        </section>
      )}

      {screen === 'todos' && (
        <section className="history-screen todo-screen">
          <header className="screen-header">
            <button className="icon-button" type="button" aria-label="返回" onClick={() => setScreen('home')}>
              <ArrowLeft />
            </button>
            <h1>待办</h1>
            <span className="header-spacer" />
          </header>
          <p className="todo-intro">从视觉记忆创建的事项；完成前可以随时回看当时的画面和摘要。</p>
          {todoBusy && <div className="history-skeleton" aria-label="正在加载"><span /><span /><span /></div>}
          {todoError && <div className="history-error">{todoError}</div>}
          {!todoBusy && !todoError && todoItems.length === 0 && (
            <div className="history-empty">
              <ListChecks />
              <h2>没有待处理事项</h2>
              <p>视频通话时说“把这个做成待办”即可把画面、摘要和任务一起保存。</p>
              <button type="button" onClick={() => openCall('video')}>打开视频通话</button>
            </div>
          )}
          {!todoBusy && todoItems.length > 0 && (
            <div className="todo-list">
              {todoItems.map((todo) => (
                <article className="todo-card" key={todo.id}>
                  <button
                    className="todo-complete"
                    type="button"
                    aria-label={`完成：${todo.title}`}
                    onClick={() => void setTodoCompleted(todo)}
                  >
                    <Circle />
                  </button>
                  {todo.cover ? (
                    <AuthenticatedImage server={server} token={accessToken} artifact={todo.cover} className="todo-cover" />
                  ) : (
                    <span className="todo-cover todo-text-cover"><CheckCircle /></span>
                  )}
                  <div>
                    <strong>{todo.title}</strong>
                    {todo.visual_summary && <p>{todo.visual_summary}</p>}
                    <time>{todo.due_at ? `提醒：${formatHistoryTime(todo.due_at)}` : '未设置提醒'}</time>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>
      )}

      {screen === 'memories' && (
        <section className="history-screen memory-screen">
          <header className="screen-header">
            <button
              className="icon-button"
              type="button"
              aria-label="返回"
              onClick={() => setScreen('home')}
            >
              <ArrowLeft />
            </button>
            <h1>视觉记忆</h1>
            <span className="header-spacer" />
          </header>

          <div className="library-region" aria-label="搜索视觉记忆">
            <LibraryToolbar
              kind="视觉记忆"
              query={memoryQuery}
              scope={memoryScope}
              selectionCount={memorySelection.size}
              onQueryChange={setMemoryQuery}
              onScopeChange={(scope) => {
                setMemoryScope(scope)
                setMemorySelection(new Set())
                setRevealedItem(null)
              }}
              onBatchAction={(action) =>
                void optimisticMemoryAction([...memorySelection], action)
              }
              onCancelSelection={() => setMemorySelection(new Set())}
            />
          </div>

          {memoryBusy && (
            <div className="history-skeleton" aria-label="正在加载">
              <span />
              <span />
              <span />
            </div>
          )}
          {memoryError && <div className="history-error">{memoryError}</div>}
          {!memoryBusy && !memoryError && memoryGroups.length === 0 && (
            <div className="history-empty">
              <ImagesSquare />
              <h2>{memoryQuery ? '没有找到相关记忆' : memoryScope === 'archived' ? '还没有归档记忆' : memoryScope === 'pinned' ? '还没有标记记忆' : '还没有保存记忆'}</h2>
              <p>{memoryQuery ? '试试搜索物品、地点或备注里的关键词。' : '视频通话时说“帮我记住这个”，我会保存当时的内容和画面。'}</p>
              {memoryQuery ? (
                <button type="button" onClick={() => setMemoryQuery('')}>清除搜索</button>
              ) : memoryScope === 'all' ? (
                <button type="button" onClick={() => openCall('video')}>打开视频通话</button>
              ) : null}
            </div>
          )}
          {!memoryBusy && memoryGroups.length > 0 && (
            <div className="library-groups memory-list">
              {memoryGroups.map((group) => (
                <LibrarySection key={group.label} label={group.label} count={group.items.length}>
                  <div className="memory-library-grid">
                    {group.items.map((libraryItem) => {
                      const memory = memoryItems.find((candidate) => candidate.id === libraryItem.id)
                      if (!memory) return null
                      const selected = memorySelection.has(memory.id)
                      return (
                        <article
                          key={memory.id}
                          className={`memory-card library-swipe-shell ${revealedItem === memory.id ? 'is-revealed' : ''}`}
                          onPointerDown={(event) =>
                            beginLibraryGesture(event, memory.id, () =>
                              toggleSelection(setMemorySelection, memory.id),
                            )
                          }
                          onPointerMove={moveLibraryGesture}
                          onPointerUp={endLibraryGesture}
                          onPointerCancel={() => {
                            cancelLongPress()
                            pointerStartRef.current = null
                          }}
                        >
                          <button
                            className="memory-card-hit library-item-surface"
                            type="button"
                            aria-pressed={selected}
                            onClick={() => {
                              if (suppressClickRef.current) {
                                suppressClickRef.current = false
                                return
                              }
                              if (memorySelection.size > 0) {
                                toggleSelection(setMemorySelection, memory.id)
                                return
                              }
                              setSelectedMemoryId(memory.id)
                            }}
                          >
                            {memory.cover ? (
                              <AuthenticatedImage server={server} token={accessToken} artifact={memory.cover} className="memory-cover" />
                            ) : (
                              <span className="memory-cover memory-text-cover"><ImagesSquare /><span>文字记忆</span></span>
                            )}
                            {memorySelection.size > 0 && (
                              <span className={`selection-check card-check ${selected ? 'is-selected' : ''}`} aria-hidden="true" />
                            )}
                            {memory.is_pinned && <PushPin className="memory-pin" weight="fill" aria-label="已标记" />}
                            <span className="memory-card-body">
                              <strong>{memory.user_note || '未命名记忆'}</strong>
                              <time>{formatHistoryTime(memory.captured_at ?? memory.created_at)}</time>
                            </span>
                          </button>
                          <LibraryActions
                            pinned={memory.is_pinned}
                            archived={memory.archived_at !== null}
                            onAction={(action) => void optimisticMemoryAction([memory.id], action)}
                          />
                        </article>
                      )
                    })}
                  </div>
                </LibrarySection>
              ))}
            </div>
          )}

          {selectedMemory && (
            <div className="memory-detail-backdrop" role="presentation" onMouseDown={(event) => {
              if (event.target === event.currentTarget) setSelectedMemoryId(null)
            }}>
              <section className="memory-detail-sheet" role="dialog" aria-modal="true" aria-labelledby="memory-detail-title">
                <header>
                  <h2 id="memory-detail-title">记忆详情</h2>
                  <button type="button" aria-label="关闭记忆详情" onClick={() => setSelectedMemoryId(null)}><X /></button>
                </header>
                {selectedMemory.cover ? (
                  <AuthenticatedImage server={server} token={accessToken} artifact={selectedMemory.cover} className="memory-detail-cover" />
                ) : (
                  <div className="memory-detail-cover memory-text-cover"><ImagesSquare /><span>文字记忆</span></div>
                )}
                <time>{formatHistoryTime(selectedMemory.captured_at ?? selectedMemory.created_at)}</time>
                {editingMemoryId === selectedMemory.id ? (
                  <textarea value={memoryDraft} maxLength={2000} autoFocus onChange={(event) => setMemoryDraft(event.target.value)} />
                ) : (
                  <h3>{selectedMemory.user_note || '未命名记忆'}</h3>
                )}
                {selectedMemory.visual_summary && <p>{selectedMemory.visual_summary}</p>}
                <div className="memory-actions">
                  {editingMemoryId === selectedMemory.id ? (
                    <>
                      <button type="button" onClick={() => void saveMemoryEdit(selectedMemory.id)}>保存</button>
                      <button type="button" onClick={() => { setEditingMemoryId(null); setMemoryDraft('') }}>取消</button>
                    </>
                  ) : (
                    <button type="button" onClick={() => { setEditingMemoryId(selectedMemory.id); setMemoryDraft(selectedMemory.user_note) }}><NotePencil /> 修改</button>
                  )}
                  <button className="danger-action" type="button" onClick={() => setDeleteRequest({ kind: 'memory', ids: [selectedMemory.id] })}><Trash /> 删除</button>
                </div>
              </section>
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
                  <MarkdownContent>{message.content}</MarkdownContent>
                  {message.attachments.length > 0 && (
                    <div className="message-attachments">
                      {message.attachments.map((artifact) => (
                        <AuthenticatedImage
                          key={artifact.id}
                          server={server}
                          token={accessToken}
                          artifact={artifact}
                          className="message-attachment"
                        />
                      ))}
                    </div>
                  )}
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
            <h1>账户设置</h1>
            <span className="header-spacer" />
          </header>

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
                <MarkdownContent>
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
                </MarkdownContent>
                {liveArtifacts.length > 0 && (
                  <div className="live-artifacts">
                    {liveArtifacts.map((artifact) => (
                      <AuthenticatedImage
                        key={artifact.id}
                        server={server}
                        token={accessToken}
                        artifact={artifact}
                        className="live-artifact"
                      />
                    ))}
                  </div>
                )}
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

      {deleteRequest && (
        <div className="confirm-dialog-backdrop" role="presentation">
          <section
            className="confirm-dialog"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="delete-dialog-title"
            aria-describedby="delete-dialog-description"
          >
            <span className="confirm-dialog-mark"><Trash aria-hidden="true" /></span>
            <h2 id="delete-dialog-title">
              删除{deleteRequest.ids.length > 1 ? `${deleteRequest.ids.length} 项` : deleteRequest.kind === 'history' ? '这段对话' : '这条记忆'}？
            </h2>
            <p id="delete-dialog-description">删除后无法恢复。视觉记忆与聊天记录会分别保留，不会连带删除。</p>
            <div>
              <button type="button" onClick={() => setDeleteRequest(null)}>取消</button>
              <button className="danger-action" type="button" autoFocus onClick={() => void confirmDelete()}>确认删除</button>
            </div>
          </section>
        </div>
      )}
    </main>
  )
}
