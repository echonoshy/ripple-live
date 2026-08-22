import {
  RotateCcw as ArrowCounterClockwise,
  ArrowLeft,
  MessageCircle as ChatCircleDots,
  Circle,
  Database,
  Mail as EnvelopeSimple,
  FolderKanban,
  Image as ImagesSquare,
  LockKeyhole as LockKey,
  ListTodo as ListChecks,
  Mic as Microphone,
  MoreVertical as DotsThreeVertical,
  Pin as PushPin,
  LogOut as SignOut,
  SquarePen as NotePencil,
  Plus,
  Search,
  Trash2 as Trash,
  Ticket,
  ChevronRight,
  Video,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import type { ChangeEvent } from 'react'
import './App.css'
import './components/AppNavigation.css'
import appIcon from '../src-tauri/icons/icon.png'
import {
  assetBlob,
  batchConversations,
  batchMemories,
  conversation,
  conversationMessages,
  conversationMutation,
  conversations,
  createTodo,
  currentUser,
  deleteTodo,
  login,
  logout as logoutApi,
  meeting,
  meetings,
  memories,
  memory,
  memoryMutation,
  renameConversation,
  regenerateMeeting,
  register,
  startMeeting,
  finishMeeting,
  promoteMeetingAction,
  uploadUserAvatar,
  updateMemory,
  todos,
  updateTodo,
  type AuthUser,
  type ConversationAction,
  type ConversationMessage,
  type ConversationSummary,
  type MemoryArtifact,
  type MeetingDetail,
  type MeetingRecord,
  type TodoItem,
  type VisualMemory,
} from './api'
import { LibraryActions } from './components/LibraryActions'
import { AvatarEditor } from './components/AvatarEditor'
import { AppDrawer, type AppDestination } from './components/AppDrawer'
import { ConversationHome } from './components/ConversationHome'
import {
  activateConversationAction,
} from './conversationActions'
import { ConversationActions } from './components/ConversationActions'
import { LiveCallScreen } from './components/LiveCallScreen'
import { LibrarySection } from './components/LibrarySection'
import { LibraryToolbar } from './components/LibraryToolbar'
import { MarkdownContent } from './components/MarkdownContent'
import { MeetingRecords } from './components/MeetingRecords'
import { PersonalizationSection } from './components/PersonalizationSection'
import { SecondaryScaffold } from './components/SecondaryScaffold'
import { UserAvatar } from './components/UserAvatar'
import {
  groupLibraryItems,
  libraryOptionsForView,
  matchesLibraryQuery,
  type LibraryAction,
  type LibraryItem,
  type LibraryView,
} from './library'
import { cameraErrorAfterSwitch, visibleCallError } from './live/callErrors'
import { createCameraActivationGuard } from './live/cameraActivation'
import {
  createCameraOrchestrator,
  type CameraPhase,
} from './live/cameraOrchestration'
import {
  closeCallBeforeDetachedRefresh,
  createCallLifecycleGuard,
  createConversationOwnership,
  createLatestNavigationGuard,
  createSingleFlight,
} from './live/callLifecycle'
import { liveResultsReducer } from './live/liveResults'
import { createMinimumVisibleSignal } from './live/frameRequestVisibility'
import {
  consumeRippleSignalsThrough,
  createRippleSignal,
  enqueueRippleSignal,
  type RippleSignal,
  type RippleSignalId,
} from './live/ripple'
import { useEdgeSwipeBack } from './edgeSwipeBack'
import { LiveMedia } from './media/LiveMedia'
import { notifyDueTodos } from './reminders'
import {
  RealtimeSession,
  type ResponseArtifact,
  type RealtimeMode,
  type SessionState,
  type LiveTranscriptTurn,
} from './realtime/RealtimeSession'
import { parseLiveResult } from './realtime/toolResults'

const DEFAULT_SERVER = '140.143.229.103:8700'
const DEFAULT_VIEWPORT = 'width=device-width, initial-scale=1.0'
const ZOOM_LOCKED_VIEWPORT = `${DEFAULT_VIEWPORT}, maximum-scale=1.0, user-scalable=no`

type Screen =
  | 'home'
  | 'call'
  | 'settings'
  | 'history'
  | 'meetings'
  | 'projects'
  | 'materials'
  | 'conversation'
  | 'memories'
  | 'todos'
  | 'personalization'

function destinationForScreen(screen: Screen): AppDestination {
  switch (screen) {
    case 'home':
    case 'call':
    case 'conversation':
      return 'home'
    case 'history':
      return 'history'
    case 'meetings':
      return 'meetings'
    case 'projects':
      return 'projects'
    case 'materials':
      return 'materials'
    case 'memories':
      return 'memories'
    case 'todos':
      return 'todos'
    case 'personalization':
      return 'personalization'
    case 'settings':
      return 'settings'
  }
}

function hasConversationContent(messages: ConversationMessage[]) {
  return messages.some(
    (message) =>
      message.content.trim().length > 0 ||
      message.attachments.length > 0 ||
      message.actions.length > 0,
  )
}

function memoryDisplayTitle(memory: VisualMemory) {
  const note = memory.user_note?.trim() ?? ''
  if (note.startsWith('系统附带了') && memory.visual_summary?.trim()) {
    return memory.visual_summary.trim()
  }
  return note || '未命名记忆'
}

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

function formatHistoryTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp * 1000))
}

function todoDueLabel(dueAt: number | null) {
  if (!dueAt) return '未设置提醒'
  const now = new Date()
  const due = new Date(dueAt * 1000)
  const startOfTomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)
  if (due.getTime() < now.getTime()) return `已逾期 · ${formatHistoryTime(dueAt)}`
  if (due < startOfTomorrow) return `今天 ${due.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })}`
  if (due < new Date(startOfTomorrow.getTime() + 86_400_000)) return `明天 ${due.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })}`
  return `提醒：${formatHistoryTime(dueAt)}`
}

function todoSummaryLabel(summary: string) {
  return summary
    .replace(/[，,]\s*当前时间为\d{4}-\d{2}-\d{2}T[^。]+。?$/u, '。')
    .replace(/。{2,}$/u, '。')
}

function todoDateInputValue(dueAt: number | null) {
  if (!dueAt) return ''
  const date = new Date(dueAt * 1000)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function notificationPermissionLabel() {
  try {
    if (typeof Notification === 'undefined') return '当前环境不可查询'
    if (Notification.permission === 'granted') return '已允许'
    if (Notification.permission === 'denied') return '已拒绝'
    return '尚未询问'
  } catch {
    return '当前环境不可查询'
  }
}

export default function App() {
  const [screen, setScreen] = useState<Screen>('home')
  const [navigationOpen, setNavigationOpen] = useState(false)
  const [mode, setMode] = useState<RealtimeMode>('audio')
  const server = DEFAULT_SERVER
  const [sessionState, setSessionState] = useState<SessionState>('idle')
  const [errorMessage, setErrorMessage] = useState('')
  const [cameraErrorMessage, setCameraErrorMessage] = useState('')
  const [cameraPhase, setCameraPhase] = useState<CameraPhase>('off')
  const [cameraPreviewVisible, setCameraPreviewVisible] = useState(false)
  const [cameraControlReady, setCameraControlReady] = useState(false)
  const [frameRequestActive, setFrameRequestActive] = useState(false)
  const [assistantText, setAssistantText] = useState('')
  const [userText, setUserText] = useState('')
  const [toolStatus, setToolStatus] = useState('')
  const [muted, setMuted] = useState(false)
  const [inputLevel, setInputLevel] = useState(0)
  const [outputLevel, setOutputLevel] = useState(0)
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
  const [avatarFile, setAvatarFile] = useState<File | null>(null)
  const [avatarBusy, setAvatarBusy] = useState(false)
  const [avatarError, setAvatarError] = useState('')
  const [avatarNotice, setAvatarNotice] = useState('')
  const [historyItems, setHistoryItems] = useState<ConversationSummary[]>([])
  const [historyMessages, setHistoryMessages] = useState<ConversationMessage[]>([])
  const [historyBusy, setHistoryBusy] = useState(false)
  const [historyError, setHistoryError] = useState('')
  const [historyQuery, setHistoryQuery] = useState('')
  const [debouncedHistoryQuery, setDebouncedHistoryQuery] = useState('')
  const [historyScope, setHistoryScope] = useState<LibraryView>('all')
  const [historySelection, setHistorySelection] = useState<Set<string>>(new Set())
  const [historySelectionMode, setHistorySelectionMode] = useState(false)
  const [selectedConversation, setSelectedConversation] = useState<ConversationSummary | null>(null)
  const [activeConversationId, setActiveConversationIdState] = useState<string | null>(null)
  const [meetingItems, setMeetingItems] = useState<MeetingRecord[]>([])
  const [selectedMeeting, setSelectedMeeting] = useState<MeetingDetail | null>(null)
  const [meetingBusy, setMeetingBusy] = useState(false)
  const [meetingError, setMeetingError] = useState('')
  const [liveTranscript, setLiveTranscript] = useState<LiveTranscriptTurn[]>([])
  const [meetingCaptureError, setMeetingCaptureError] = useState('')
  const [memoryItems, setMemoryItems] = useState<VisualMemory[]>([])
  const [memoryBusy, setMemoryBusy] = useState(false)
  const [memoryError, setMemoryError] = useState('')
  const [memoryQuery, setMemoryQuery] = useState('')
  const [debouncedMemoryQuery, setDebouncedMemoryQuery] = useState('')
  const [memoryScope, setMemoryScope] = useState<LibraryView>('all')
  const [memorySelection, setMemorySelection] = useState<Set<string>>(new Set())
  const [memorySelectionMode, setMemorySelectionMode] = useState(false)
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null)
  const [editingMemoryId, setEditingMemoryId] = useState<string | null>(null)
  const [memoryDraft, setMemoryDraft] = useState('')
  const [todoItems, setTodoItems] = useState<TodoItem[]>([])
  const [todoView, setTodoView] = useState<'active' | 'completed'>('active')
  const [todoQuery, setTodoQuery] = useState('')
  const [todoEditor, setTodoEditor] = useState<{ todo?: TodoItem; title: string; dueAt: string } | null>(null)
  const [todoBusy, setTodoBusy] = useState(false)
  const [todoError, setTodoError] = useState('')
  const [revealedTodo, setRevealedTodo] = useState<string | null>(null)
  const [todoDrag, setTodoDrag] = useState<{ id: string; offset: number } | null>(null)
  const [revealedItem, setRevealedItem] = useState<string | null>(null)
  const [deleteRequest, setDeleteRequest] = useState<{
    kind: 'history' | 'memory' | 'todo'
    ids: string[]
  } | null>(null)
  const [renameRequest, setRenameRequest] = useState<ConversationSummary | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const [renameBusy, setRenameBusy] = useState(false)
  const [renameError, setRenameError] = useState('')
  const [liveArtifacts, setLiveArtifacts] = useState<ResponseArtifact[]>([])
  const [liveResults, dispatchLiveResults] = useReducer(liveResultsReducer, [])
  const [rippleSignals, setRippleSignals] = useState<readonly RippleSignal[]>([])

  const videoRef = useRef<HTMLVideoElement>(null)
  const appShellRef = useRef<HTMLElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const avatarInputRef = useRef<HTMLInputElement>(null)
  const sessionRef = useRef<RealtimeSession | null>(null)
  const mediaRef = useRef<LiveMedia | null>(null)
  const cameraOrchestratorRef = useRef<ReturnType<
    typeof createCameraOrchestrator
  > | null>(null)
  const frameRequestVisibilityRef = useRef<ReturnType<
    typeof createMinimumVisibleSignal
  > | null>(null)
  const cameraControlReadyRef = useRef(false)
  const cameraActivationInvalidatorRef = useRef<(() => void) | null>(null)
  const initialCameraRequestRef = useRef(false)
  const activeMeetingIdRef = useRef<string | null>(null)
  const meetingStartPromiseRef = useRef<Promise<string | null> | null>(null)
  const cameraFlipGenerationRef = useRef(0)
  const callLifecycleRef = useRef(createCallLifecycleGuard())
  const conversationOwnershipRef = useRef(createConversationOwnership())
  const navigationGuardRef = useRef(createLatestNavigationGuard())
  const preloadedConversationIdRef = useRef<string | null>(null)
  const longPressTimerRef = useRef<number | null>(null)
  const pointerStartRef = useRef<{ id: string; x: number; y: number } | null>(null)
  const todoPointerStartRef = useRef<{
    id: string
    x: number
    y: number
    baseOffset: number
    dragging: boolean
  } | null>(null)
  const suppressTodoClickRef = useRef(false)
  const suppressClickRef = useRef(false)
  const actionMemoryTargetRef = useRef<string | null>(null)
  const actionTodoTargetRef = useRef<string | null>(null)
  const todoScrollTargetRef = useRef<string | null>(null)

  const navigateTo = useCallback((nextScreen: Screen) => {
    const owner = navigationGuardRef.current.begin()
    setNavigationOpen(false)
    setScreen(nextScreen)
    window.requestAnimationFrame(() => window.scrollTo(0, 0))
    return owner
  }, [])

  const onRippleSignalsConsumed = useCallback((signalId: RippleSignalId) => {
    setRippleSignals((current) => consumeRippleSignalsThrough(current, signalId))
  }, [])

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedHistoryQuery(historyQuery), 250)
    return () => window.clearTimeout(timer)
  }, [historyQuery])

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedMemoryQuery(memoryQuery), 250)
    return () => window.clearTimeout(timer)
  }, [memoryQuery])

  useEffect(() => {
    if (screen !== 'conversation') return

    const viewport = document.querySelector('meta[name="viewport"]')
    const previousViewport = viewport?.getAttribute('content')
    viewport?.setAttribute('content', ZOOM_LOCKED_VIEWPORT)

    const preventGestureZoom = (event: Event) => event.preventDefault()
    const preventShortcutZoom = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) event.preventDefault()
    }
    document.addEventListener('gesturestart', preventGestureZoom, { passive: false })
    document.addEventListener('gesturechange', preventGestureZoom, { passive: false })
    document.addEventListener('gestureend', preventGestureZoom, { passive: false })
    document.addEventListener('wheel', preventShortcutZoom, { passive: false })

    return () => {
      viewport?.setAttribute('content', previousViewport ?? DEFAULT_VIEWPORT)
      document.removeEventListener('gesturestart', preventGestureZoom)
      document.removeEventListener('gesturechange', preventGestureZoom)
      document.removeEventListener('gestureend', preventGestureZoom)
      document.removeEventListener('wheel', preventShortcutZoom)
    }
  }, [screen])

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
        callLifecycleRef.current.invalidate()
        navigationGuardRef.current.invalidate()
        conversationOwnershipRef.current.invalidate()
        cameraOrchestratorRef.current?.invalidate()
        cameraOrchestratorRef.current = null
        cameraActivationInvalidatorRef.current?.()
        cameraActivationInvalidatorRef.current = null
        frameRequestVisibilityRef.current?.dispose()
        frameRequestVisibilityRef.current = null
        cameraControlReadyRef.current = false
        cameraFlipGenerationRef.current += 1
        mediaRef.current?.stop()
        mediaRef.current = null
        const liveSession = sessionRef.current
        sessionRef.current = null
        void liveSession?.close()
        initialCameraRequestRef.current = false
        setCameraPhase('off')
        setCameraPreviewVisible(false)
        setCameraControlReady(false)
        setFrameRequestActive(false)
        setActiveConversationIdState(null)
        setSelectedConversation(null)
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
    if ((screen !== 'home' && screen !== 'history') || !accessToken) return
    let active = true
    if (screen === 'history') setHistoryBusy(true)
    setHistoryError('')
    void conversations(
      server,
      accessToken,
      screen === 'home'
        ? libraryOptionsForView('all', '', 1)
        : libraryOptionsForView(historyScope, debouncedHistoryQuery, 100),
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
        if (active && screen === 'history') setHistoryBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, debouncedHistoryQuery, historyScope, screen, server])

  useEffect(() => {
    if (screen !== 'conversation' || !accessToken || !selectedConversation) return
    if (preloadedConversationIdRef.current === selectedConversation.id) {
      preloadedConversationIdRef.current = null
      return
    }
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
        if (!active) return
        const targetId = actionMemoryTargetRef.current
        setMemoryItems((current) => {
          const focused = targetId
            ? current.find((item) => item.id === targetId)
            : null
          return focused && !items.some((item) => item.id === focused.id)
            ? [focused, ...items]
            : items
        })
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
    setTodoItems([])
    const targetId = actionTodoTargetRef.current
    const load = async () => {
      if (!targetId) return todos(server, accessToken, todoView === 'completed')
      const activeItems = await todos(server, accessToken, false)
      if (activeItems.some((item) => item.id === targetId)) {
        return activeItems
      }
      const completedItems = await todos(server, accessToken, true)
      if (completedItems.some((item) => item.id === targetId)) {
        if (active) setTodoView('completed')
        return completedItems
      }
      if (active) setTodoError('该待办已不存在或无法打开')
      return activeItems
    }
    void load()
      .then((items) => {
        if (!active) return
        if (targetId) {
          actionTodoTargetRef.current = null
          if (items.some((item) => item.id === targetId)) {
            todoScrollTargetRef.current = targetId
          }
        }
        setTodoItems(items)
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
  }, [accessToken, screen, server, todoView])

  useEffect(() => {
    if (screen !== 'meetings' || selectedMeeting || !accessToken) return
    let active = true
    setMeetingBusy(true)
    setMeetingError('')
    void meetings(server, accessToken)
      .then((items) => {
        if (active) setMeetingItems(items)
      })
      .catch((error: unknown) => {
        if (active) setMeetingError(error instanceof Error ? error.message : '无法读取会议记录')
      })
      .finally(() => {
        if (active) setMeetingBusy(false)
      })
    return () => {
      active = false
    }
  }, [accessToken, screen, selectedMeeting, server])

  useEffect(() => {
    if (
      screen !== 'meetings' ||
      !selectedMeeting ||
      (selectedMeeting.status !== 'processing' && selectedMeeting.status !== 'recording') ||
      !accessToken
    ) return
    let active = true
    const refresh = () => {
      void meeting(server, accessToken, selectedMeeting.id)
        .then((detail) => {
          if (!active) return
          setSelectedMeeting(detail)
          setMeetingItems((items) => items.map((item) => item.id === detail.id ? detail : item))
        })
        .catch(() => {})
    }
    const timer = window.setInterval(refresh, 1_500)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [accessToken, screen, selectedMeeting, server])

  useEffect(() => {
    const targetId = todoScrollTargetRef.current
    if (screen !== 'todos' || !targetId) return
    const frame = window.requestAnimationFrame(() => {
      const target = document.getElementById(`todo-action-${encodeURIComponent(targetId)}`)
      if (!target) return
      todoScrollTargetRef.current = null
      const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
      target.scrollIntoView({ behavior: reducedMotion ? 'auto' : 'smooth', block: 'center' })
      target.focus({ preventScroll: true })
    })
    return () => window.cancelAnimationFrame(frame)
  }, [screen, todoItems])

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
    cameraOrchestratorRef.current?.invalidate()
    cameraOrchestratorRef.current = null
    cameraActivationInvalidatorRef.current?.()
    cameraActivationInvalidatorRef.current = null
    frameRequestVisibilityRef.current?.dispose()
    frameRequestVisibilityRef.current = null
    cameraControlReadyRef.current = false
    cameraFlipGenerationRef.current += 1
    mediaRef.current?.stop()
    mediaRef.current = null
    setCameraPhase('off')
    setCameraPreviewVisible(false)
    setCameraControlReady(false)
    setFrameRequestActive(false)
    setCameraErrorMessage('')
    setInputLevel(0)
    setOutputLevel(0)
  }, [sessionState])

  const stopCall = useCallback(async () => {
    cameraOrchestratorRef.current?.invalidate()
    cameraOrchestratorRef.current = null
    cameraActivationInvalidatorRef.current?.()
    cameraActivationInvalidatorRef.current = null
    frameRequestVisibilityRef.current?.dispose()
    frameRequestVisibilityRef.current = null
    cameraControlReadyRef.current = false
    cameraFlipGenerationRef.current += 1
    mediaRef.current?.stop()
    mediaRef.current = null
    initialCameraRequestRef.current = false
    setCameraPhase('off')
    setCameraPreviewVisible(false)
    setCameraControlReady(false)
    setFrameRequestActive(false)
    setCameraErrorMessage('')
    setInputLevel(0)
    setOutputLevel(0)
    setRippleSignals([])
    const session = sessionRef.current
    sessionRef.current = null
    dispatchLiveResults({ type: 'clear' })
    await session?.close()
  }, [])

  const leaveCall = useMemo(
    () =>
      createSingleFlight(async () => {
        if (!callLifecycleRef.current.beginLeave()) return
        const conversationOwner = conversationOwnershipRef.current.current()
        const routeOwner = navigateTo('home')
        const token = accessToken
        const meetingStart = meetingStartPromiseRef.current
        await closeCallBeforeDetachedRefresh({
          close: async () => {
            try {
              await stopCall()
            } finally {
              if (
                conversationOwnershipRef.current.release(
                  conversationOwner.owner,
                )
              ) {
                setActiveConversationIdState(null)
              }
            }
          },
          finishClose: () => {
            callLifecycleRef.current.finishLeave()
            setSessionState('idle')
          },
          refresh: async () => {
            const meetingId = await meetingStart
            activeMeetingIdRef.current = null
            meetingStartPromiseRef.current = null
            if (meetingId && token) {
              try {
                await finishMeeting(server, token, meetingId)
                const detail = await meeting(server, token, meetingId)
                if (!navigationGuardRef.current.owns(routeOwner)) return
                setMeetingError('')
                setSelectedMeeting(detail)
                setMeetingItems((items) => [detail, ...items.filter((item) => item.id !== detail.id)])
                setScreen('meetings')
                return
              } catch (error) {
                if (!navigationGuardRef.current.owns(routeOwner)) return
                setMeetingError(
                  `通话已保存，但会议整理未启动：${error instanceof Error ? error.message : '请稍后重试'}`,
                )
                setSelectedMeeting(null)
                setScreen('meetings')
                return
              }
            }
            if (!conversationOwner.conversationId || !token) return
            try {
              const [summary, messages] = await Promise.all([
                conversation(server, token, conversationOwner.conversationId),
                conversationMessages(
                  server,
                  token,
                  conversationOwner.conversationId,
                ),
              ])
              if (!navigationGuardRef.current.owns(routeOwner)) return
              setHistoryError('')
              setHistoryBusy(false)
              if (!hasConversationContent(messages)) {
                setSelectedConversation(null)
                setHistoryMessages([])
                preloadedConversationIdRef.current = null
                setScreen('home')
                return
              }
              setSelectedConversation(summary)
              setHistoryMessages(messages)
              preloadedConversationIdRef.current = summary.id
              setScreen('conversation')
            } catch (error) {
              if (!navigationGuardRef.current.owns(routeOwner)) return
              setSelectedConversation(null)
              setHistoryMessages([])
              setHistoryBusy(false)
              setHistoryError(
                `通话已结束，但无法刷新聊天记录：${
                  error instanceof Error ? error.message : '请稍后重试'
                }`,
              )
              setScreen('home')
            }
          },
        })
      }),
    [accessToken, navigateTo, server, stopCall],
  )

  const handleEdgeSwipeBack = useCallback(() => {
    if (deleteRequest) {
      setDeleteRequest(null)
      return
    }
    if (renameRequest) {
      setRenameRequest(null)
      setRenameDraft('')
      setRenameError('')
      return
    }
    if (todoEditor) {
      setTodoEditor(null)
      return
    }
    if (selectedMemoryId) {
      setSelectedMemoryId(null)
      setEditingMemoryId(null)
      setMemoryDraft('')
      return
    }
    if (historySelectionMode) {
      setHistorySelection(new Set())
      setHistorySelectionMode(false)
      return
    }
    if (memorySelectionMode) {
      setMemorySelection(new Set())
      setMemorySelectionMode(false)
      return
    }
    if (revealedItem) {
      setRevealedItem(null)
      return
    }
    if (revealedTodo) {
      setRevealedTodo(null)
      return
    }
    if (screen === 'call') {
      void leaveCall()
      return
    }
    if (screen === 'conversation') {
      navigateTo('history')
      return
    }
    if (screen !== 'home') navigateTo('home')
  }, [
    deleteRequest,
    historySelectionMode,
    leaveCall,
    memorySelectionMode,
    navigateTo,
    renameRequest,
    revealedItem,
    revealedTodo,
    screen,
    selectedMemoryId,
    todoEditor,
  ])

  const edgeSwipeBackEnabled = !navigationOpen && (
    screen !== 'home' ||
    Boolean(deleteRequest || renameRequest || todoEditor || selectedMemoryId)
  )

  useEdgeSwipeBack({
    rootRef: appShellRef,
    enabled: edgeSwipeBackEnabled,
    onBack: handleEdgeSwipeBack,
  })

  useEffect(() => {
    const callLifecycle = callLifecycleRef.current
    const conversationOwnership = conversationOwnershipRef.current
    const navigationGuard = navigationGuardRef.current
    return () => {
      const media = mediaRef.current
      const session = sessionRef.current
      callLifecycle.invalidate()
      conversationOwnership.invalidate()
      navigationGuard.invalidate()
      cameraOrchestratorRef.current?.invalidate()
      cameraOrchestratorRef.current = null
      cameraActivationInvalidatorRef.current?.()
      cameraActivationInvalidatorRef.current = null
      frameRequestVisibilityRef.current?.dispose()
      frameRequestVisibilityRef.current = null
      cameraControlReadyRef.current = false
      cameraFlipGenerationRef.current += 1
      mediaRef.current = null
      sessionRef.current = null
      media?.stop()
      void session?.close()
    }
  }, [])

  const startCall = useCallback(
    async (owner: number) => {
      if (
        !callLifecycleRef.current.owns(owner) ||
        sessionRef.current ||
        !videoRef.current ||
        !canvasRef.current
      ) {
        callLifecycleRef.current.fail(owner)
        return
      }

      const requestedMeetingMode: RealtimeMode = initialCameraRequestRef.current ? 'video' : 'audio'
      setMode('audio')
      setErrorMessage('')
      setCameraErrorMessage('')
      setAssistantText('')
      setUserText('')
      setToolStatus('')
      setLiveArtifacts([])
      dispatchLiveResults({ type: 'clear' })
      setRippleSignals([])
      setLiveTranscript([])
      setMeetingCaptureError('')
      activeMeetingIdRef.current = null
      meetingStartPromiseRef.current = null
      setElapsed(0)
      setMuted(false)
      setInputLevel(0)
      setOutputLevel(0)
      cameraControlReadyRef.current = false
      setCameraControlReady(false)
      setSessionState('connecting')

      const cameraActivation = createCameraActivationGuard(
        initialCameraRequestRef.current,
      )
      initialCameraRequestRef.current = false
      cameraActivation.transition('connecting')
      const invalidateCameraActivation = () => cameraActivation.invalidate()
      let session: RealtimeSession
      let cameraOrchestrator: ReturnType<typeof createCameraOrchestrator>
      let frameRequestVisibility: ReturnType<typeof createMinimumVisibleSignal>
      const conversationOwner = conversationOwnershipRef.current.current()
      const ownsSession = () =>
        callLifecycleRef.current.owns(owner) &&
        sessionRef.current === session
      const setActiveConversationId = (conversationId: string) => {
        if (!ownsSession()) return
        const confirmedConversationId =
          conversationOwnershipRef.current.confirm(
            conversationOwner.owner,
            conversationId,
          )
        if (confirmedConversationId) {
          setActiveConversationIdState(confirmedConversationId)
          if (!meetingStartPromiseRef.current) {
            const start = startMeeting(
              server,
              accessToken,
              confirmedConversationId,
              requestedMeetingMode,
            )
              .then((record) => {
                if (!ownsSession()) return record.id
                activeMeetingIdRef.current = record.id
                return record.id
              })
              .catch((error: unknown) => {
                if (ownsSession()) {
                  setMeetingCaptureError(
                    `会议记录未启动：${error instanceof Error ? error.message : '请稍后重试'}`,
                  )
                }
                return null
              })
            meetingStartPromiseRef.current = start
          }
        }
      }
      const media = new LiveMedia({
        video: videoRef.current,
        canvas: canvasRef.current,
        facingMode: cameraFacing,
        onPlaybackStarted: (bufferedMs) => {
          if (ownsSession()) session.outputPlaybackStarted(bufferedMs)
        },
        onPlaybackEnded: () => {
          if (ownsSession()) session.outputPlaybackEnded()
        },
        onOutputLevel: (level) => {
          if (ownsSession()) setOutputLevel(level)
        },
        onCameraInterrupted: () => {
          if (ownsSession()) void cameraOrchestrator.interrupt()
        },
      })
      session = new RealtimeSession({
        server,
        accessToken,
        conversationId: activeConversationId ?? undefined,
        mode: 'audio',
        onState: (state) => {
          cameraActivation.transition(state)
          if (!ownsSession()) return
          if (
            state === 'idle' ||
            state === 'connecting' ||
            state === 'preparing' ||
            state === 'ended' ||
            state === 'error'
          ) {
            cameraControlReadyRef.current = false
            setCameraControlReady(false)
          }
          setSessionState(state)
        },
        onError: (message) => {
          cameraActivation.invalidate()
          if (!ownsSession()) return
          cameraControlReadyRef.current = false
          setCameraControlReady(false)
          setInputLevel(0)
          setOutputLevel(0)
          setErrorMessage(message)
          dispatchLiveResults({ type: 'clear' })
          setSessionState('error')
        },
        onResponseFailed: (message) => {
          if (!ownsSession()) return
          setErrorMessage(message)
        },
        onAssistantText: (text) => {
          if (ownsSession()) setAssistantText(text)
        },
        onUserText: (text) => {
          if (!ownsSession()) return
          setUserText(text)
          setLiveArtifacts([])
          dispatchLiveResults({ type: 'clear' })
        },
        onTranscriptTurn: (turn) => {
          if (ownsSession()) setLiveTranscript((items) => [...items, turn].slice(-200))
        },
        onTool: (status) => {
          if (ownsSession()) setToolStatus(status)
        },
        onToolResult: (event) => {
          if (!ownsSession()) return
          dispatchLiveResults({ type: 'add', result: parseLiveResult(event) })
          const signal = createRippleSignal('tool')
          setRippleSignals((current) => enqueueRippleSignal(current, signal))
        },
        onAudio: (audio) => {
          if (ownsSession()) media.enqueueOutput(audio)
        },
        onAudioDone: () => {
          if (ownsSession()) media.finishOutput()
        },
        onInterrupted: () => {
          if (!ownsSession()) return
          media.clearOutput()
          const signal = createRippleSignal('interrupt')
          setRippleSignals((current) => enqueueRippleSignal(current, signal))
        },
        onFrameRequested: () =>
          ownsSession() ? media.captureFrame() : null,
        onFrameRequestState: (active) => {
          if (ownsSession()) frameRequestVisibility.update(active)
        },
        onArtifact: (artifact) => {
          if (!ownsSession()) return
          setLiveArtifacts((items) =>
            items.some((item) => item.id === artifact.id)
              ? items
              : [...items, artifact],
          )
        },
        onConversation: setActiveConversationId,
        onReady: async () => {
          if (!ownsSession()) return
          const activationToken = cameraActivation.begin()
          if (activationToken === null) return
          await media.start((audio) => {
            if (ownsSession()) void session.sendInput(audio)
          }, () => {
            if (!ownsSession()) return
            setUserText('')
            setAssistantText('')
            const signal = createRippleSignal('speech')
            setRippleSignals((current) => enqueueRippleSignal(current, signal))
            void session.speechStarted()
          }, () => {
            if (ownsSession()) void session.speechPaused()
          }, (level) => {
            if (ownsSession()) setInputLevel(level)
          })
          const exactResources =
            mediaRef.current === media &&
            cameraOrchestratorRef.current === cameraOrchestrator
          const activation = exactResources && ownsSession()
            ? cameraActivation.commit(activationToken)
            : null
          if (!activation) return
          cameraControlReadyRef.current = true
          setCameraControlReady(true)
          if (activation.cameraRequested) {
            await cameraOrchestrator.open(cameraFacing)
          }
        },
      })

      cameraOrchestrator = createCameraOrchestrator({
        enableCamera: (facingMode) => media.enableCamera(facingMode),
        disableCamera: () => media.disableCamera(),
        setMode: (targetMode) => session.setMode(targetMode),
        waitForTransition: () =>
          window.matchMedia('(prefers-reduced-motion: reduce)').matches
            ? Promise.resolve()
            : new Promise((resolve) => window.setTimeout(resolve, 420)),
        onSnapshot: (snapshot) => {
          if (!ownsSession()) return
          setCameraPhase(snapshot.phase)
          setCameraPreviewVisible(snapshot.previewVisible)
          if (snapshot.serverMode === 'audio' || snapshot.serverMode === 'video') {
            setMode(snapshot.serverMode)
          }
        },
        onError: (message) => {
          if (ownsSession()) setCameraErrorMessage(message)
        },
      })
      frameRequestVisibility = createMinimumVisibleSignal({
        minimumMs: 160,
        onVisible: (visible) => {
          if (ownsSession()) setFrameRequestActive(visible)
        },
        timers: {
          now: () => performance.now(),
          setTimeout: (callback, delayMs) =>
            window.setTimeout(callback, delayMs),
          clearTimeout: (handle) => window.clearTimeout(handle as number),
        },
      })

      mediaRef.current = media
      sessionRef.current = session
      cameraOrchestratorRef.current = cameraOrchestrator
      frameRequestVisibilityRef.current = frameRequestVisibility
      cameraActivationInvalidatorRef.current = invalidateCameraActivation

      try {
        await session.connect()
      } catch (error) {
        if (
          sessionRef.current !== session ||
          !callLifecycleRef.current.fail(owner)
        ) {
          return
        }
        sessionRef.current = null
        mediaRef.current = null
        cameraOrchestrator.invalidate()
        if (cameraOrchestratorRef.current === cameraOrchestrator) {
          cameraOrchestratorRef.current = null
        }
        cameraActivation.invalidate()
        if (
          cameraActivationInvalidatorRef.current === invalidateCameraActivation
        ) {
          cameraActivationInvalidatorRef.current = null
        }
        frameRequestVisibility.dispose()
        if (frameRequestVisibilityRef.current === frameRequestVisibility) {
          frameRequestVisibilityRef.current = null
        }
        cameraControlReadyRef.current = false
        setCameraControlReady(false)
        initialCameraRequestRef.current = false
        media.stop()
        const closing = session.close()
        setInputLevel(0)
        setOutputLevel(0)
        const message =
          error instanceof Error ? error.message : '无法连接实时服务'
        setErrorMessage(message)
        dispatchLiveResults({ type: 'clear' })
        setSessionState('error')
        await closing
      }
    },
    [accessToken, activeConversationId, cameraFacing, server],
  )

  useEffect(() => {
    if (
      screen !== 'call' ||
      sessionRef.current ||
      !callLifecycleRef.current.canAutoStart()
    ) {
      return
    }
    const frame = window.requestAnimationFrame(() => {
      const owner = callLifecycleRef.current.claimStart()
      if (owner !== null) void startCall(owner)
    })
    return () => window.cancelAnimationFrame(frame)
  }, [screen, startCall])

  const openCall = (
    nextMode: RealtimeMode,
    conversationId?: string,
  ) => {
    if (!callLifecycleRef.current.requestOpen()) return
    const conversationOwner = conversationOwnershipRef.current.begin(conversationId)
    setActiveConversationIdState(conversationOwner.conversationId)
    if (!conversationOwner.conversationId) {
      setSelectedConversation(null)
      setHistoryMessages([])
    }
    setHistoryBusy(false)
    setHistoryError('')
    initialCameraRequestRef.current = nextMode === 'video'
    setMode('audio')
    setCameraPhase('off')
    setCameraPreviewVisible(false)
    cameraControlReadyRef.current = false
    setCameraControlReady(false)
    setFrameRequestActive(false)
    setCameraErrorMessage('')
    setSessionState('idle')
    dispatchLiveResults({ type: 'clear' })
    setRippleSignals([])
    navigateTo('call')
  }

  const openConversationMemory = async (targetId: string) => {
    const routeOwner = navigateTo('memories')
    actionMemoryTargetRef.current = targetId
    setMemoryScope('all')
    setMemoryQuery('')
    setSelectedMemoryId(null)
    setMemoryBusy(true)
    setMemoryError('')
    try {
      const target = await memory(server, accessToken, targetId)
      if (!navigationGuardRef.current.owns(routeOwner)) return
      setMemoryItems((items) => [
        target,
        ...items.filter((item) => item.id !== target.id),
      ])
      setSelectedMemoryId(target.id)
    } catch (error) {
      if (!navigationGuardRef.current.owns(routeOwner)) return
      actionMemoryTargetRef.current = null
      setMemoryError(
        error instanceof Error ? error.message : '该记忆已不存在或无法打开',
      )
    } finally {
      if (navigationGuardRef.current.owns(routeOwner)) setMemoryBusy(false)
    }
  }

  const openConversationTodo = (targetId: string) => {
    actionTodoTargetRef.current = targetId
    setTodoView('active')
    setTodoQuery('')
    setRevealedTodo(null)
    navigateTo('todos')
  }

  const openConversationAction = (action: ConversationAction) =>
    activateConversationAction(action, {
      openMemory: openConversationMemory,
      openTodo: openConversationTodo,
    })

  const openMeetingRecord = async (meetingId: string) => {
    const routeOwner = navigateTo('meetings')
    setMeetingBusy(true)
    setMeetingError('')
    try {
      const detail = await meeting(server, accessToken, meetingId)
      if (navigationGuardRef.current.owns(routeOwner)) setSelectedMeeting(detail)
    } catch (error) {
      if (navigationGuardRef.current.owns(routeOwner)) {
        setMeetingError(error instanceof Error ? error.message : '无法打开会议记录')
      }
    } finally {
      if (navigationGuardRef.current.owns(routeOwner)) setMeetingBusy(false)
    }
  }

  const retryMeetingGeneration = async (meetingId: string) => {
    setMeetingError('')
    try {
      await regenerateMeeting(server, accessToken, meetingId)
      const detail = await meeting(server, accessToken, meetingId)
      setSelectedMeeting(detail)
    } catch (error) {
      setMeetingError(error instanceof Error ? error.message : '无法重新生成会议记录')
    }
  }

  const addMeetingActionToTodos = async (meetingId: string, actionId: string) => {
    setMeetingError('')
    try {
      const action = await promoteMeetingAction(server, accessToken, meetingId, actionId)
      setSelectedMeeting((current) => current && current.id === meetingId ? {
        ...current,
        action_items: current.action_items.map((item) => item.id === action.id ? action : item),
      } : current)
    } catch (error) {
      setMeetingError(error instanceof Error ? error.message : '无法加入待办')
    }
  }

  const selectDestination = (destination: AppDestination) => {
    switch (destination) {
      case 'home':
        navigateTo('home')
        break
      case 'history':
        navigateTo('history')
        break
      case 'meetings':
        setSelectedMeeting(null)
        navigateTo('meetings')
        break
      case 'projects':
        navigateTo('projects')
        break
      case 'materials':
        navigateTo('materials')
        break
      case 'memories':
        actionMemoryTargetRef.current = null
        navigateTo('memories')
        break
      case 'todos':
        actionTodoTargetRef.current = null
        navigateTo('todos')
        break
      case 'personalization':
        navigateTo('personalization')
        break
      case 'settings':
        navigateTo('settings')
        break
    }
  }

  const toggleMute = () => {
    const next = !muted
    setMuted(next)
    mediaRef.current?.setMuted(next)
    if (next) sessionRef.current?.discardInput()
  }

  const toggleCamera = async () => {
    if (!cameraControlReadyRef.current) return
    const orchestrator = cameraOrchestratorRef.current
    if (!orchestrator) return
    setCameraErrorMessage('')
    const snapshot = orchestrator.current()
    if (snapshot.recovery) {
      await orchestrator.retry(cameraFacing)
      return
    }
    if (snapshot.previewVisible) {
      await orchestrator.close()
      return
    }
    await orchestrator.open(cameraFacing)
  }

  const flipCamera = async () => {
    const media = mediaRef.current
    const orchestrator = cameraOrchestratorRef.current
    if (
      !cameraControlReadyRef.current ||
      !media ||
      !orchestrator ||
      orchestrator.current().phase !== 'on'
    ) return
    const flipGeneration = ++cameraFlipGenerationRef.current
    const next = cameraFacing === 'user' ? 'environment' : 'user'
    setCameraErrorMessage('')
    try {
      const outcome = await media.setFacingMode(next)
      if (
        mediaRef.current !== media ||
        cameraOrchestratorRef.current !== orchestrator ||
        cameraFlipGenerationRef.current !== flipGeneration ||
        orchestrator.current().phase !== 'on'
      ) return
      if (outcome === 'stale') return
      if (outcome === 'failed') {
        setCameraErrorMessage((previous) =>
          cameraErrorAfterSwitch(previous, outcome),
        )
        return
      }
      setCameraFacing(next)
      setCameraErrorMessage((previous) =>
        cameraErrorAfterSwitch(previous, outcome),
      )
    } catch {
      if (
        mediaRef.current === media &&
        cameraOrchestratorRef.current === orchestrator &&
        cameraFlipGenerationRef.current === flipGeneration &&
        orchestrator.current().phase === 'on'
      ) {
        setCameraErrorMessage((previous) =>
          cameraErrorAfterSwitch(previous, 'failed'),
        )
      }
    }
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
    callLifecycleRef.current.invalidate()
    cameraOrchestratorRef.current?.invalidate()
    cameraOrchestratorRef.current = null
    cameraActivationInvalidatorRef.current?.()
    cameraActivationInvalidatorRef.current = null
    frameRequestVisibilityRef.current?.dispose()
    frameRequestVisibilityRef.current = null
    cameraControlReadyRef.current = false
    cameraFlipGenerationRef.current += 1
    mediaRef.current?.stop()
    mediaRef.current = null
    const liveSession = sessionRef.current
    sessionRef.current = null
    void liveSession?.close()
    initialCameraRequestRef.current = false
    setCameraPhase('off')
    setCameraPreviewVisible(false)
    setCameraControlReady(false)
    setFrameRequestActive(false)
    localStorage.removeItem('ripple-access-token')
    setAccessToken('')
    setUser(null)
    setAvatarFile(null)
    setAvatarError('')
    setAvatarNotice('')
    conversationOwnershipRef.current.invalidate()
    setActiveConversationIdState(null)
    setSelectedConversation(null)
    navigateTo('home')
    setHistoryItems([])
    setHistoryMessages([])
    setMemoryItems([])
    setTodoItems([])
    if (token) await logoutApi(server, token).catch(() => {})
  }

  function selectAvatarFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    setAvatarNotice('')
    setAvatarError('')
    if (file.size > 12 * 1024 * 1024) {
      setAvatarNotice('图片不能超过 12MB')
      return
    }
    if (file.type && !['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) {
      setAvatarNotice('请选择 JPEG、PNG 或 WebP 图片')
      return
    }
    setAvatarFile(file)
  }

  async function saveAvatar(blob: Blob) {
    if (!accessToken) return
    setAvatarBusy(true)
    setAvatarError('')
    try {
      const updatedUser = await uploadUserAvatar(server, accessToken, blob)
      setUser(updatedUser)
      setAvatarFile(null)
      setAvatarNotice('头像已更新')
    } catch (error) {
      setAvatarError(error instanceof Error ? error.message : '头像上传失败，请重试')
    } finally {
      setAvatarBusy(false)
    }
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
    try {
      await updateTodo(server, accessToken, todo.id, { completed: todoView === 'active' })
      setTodoItems((items) => items.filter((item) => item.id !== todo.id))
      setRevealedTodo(null)
    } catch (error) {
      setTodoError(error instanceof Error ? error.message : '无法更新待办')
    }
  }

  const saveTodo = async () => {
    if (!todoEditor) return
    const title = todoEditor.title.trim()
    if (!title) {
      setTodoError('请填写待办事项')
      return
    }
    const dueAt = todoEditor.dueAt ? new Date(todoEditor.dueAt).getTime() / 1000 : undefined
    if (todoEditor.dueAt && !Number.isFinite(dueAt)) {
      setTodoError('提醒时间无效')
      return
    }
    setTodoError('')
    try {
      if (todoEditor.todo) {
        const updated = await updateTodo(server, accessToken, todoEditor.todo.id, {
          title,
          due_at: dueAt,
          clear_due_at: !dueAt,
        })
        setTodoItems((items) => items.map((item) => (item.id === updated.id ? updated : item)))
      } else {
        const created = await createTodo(server, accessToken, title, dueAt)
        if (todoView === 'active') setTodoItems((items) => [created, ...items])
      }
      setTodoEditor(null)
    } catch (error) {
      setTodoError(error instanceof Error ? error.message : '无法保存待办')
    }
  }

  const beginTodoGesture = (event: React.PointerEvent<HTMLElement>, id: string) => {
    if ((event.target as HTMLElement).closest('.todo-swipe-delete, input, textarea, select, label')) return
    todoPointerStartRef.current = {
      id,
      x: event.clientX,
      y: event.clientY,
      baseOffset: revealedTodo === id ? 74 : 0,
      dragging: false,
    }
  }

  const moveTodoGesture = (event: React.PointerEvent<HTMLElement>) => {
    const start = todoPointerStartRef.current
    if (!start) return
    const deltaX = event.clientX - start.x
    const deltaY = event.clientY - start.y
    if (!start.dragging) {
      if (Math.abs(deltaX) < 7 && Math.abs(deltaY) < 7) return
      if (Math.abs(deltaY) > Math.abs(deltaX)) {
        todoPointerStartRef.current = null
        return
      }
      start.dragging = true
      event.currentTarget.setPointerCapture?.(event.pointerId)
    }
    event.preventDefault()
    const offset = Math.min(74, Math.max(0, start.baseOffset + deltaX))
    setTodoDrag({ id: start.id, offset })
  }

  const endTodoGesture = (event: React.PointerEvent<HTMLElement>) => {
    const start = todoPointerStartRef.current
    todoPointerStartRef.current = null
    if (!start) return
    const offset = Math.min(74, Math.max(0, start.baseOffset + event.clientX - start.x))
    if (start.dragging) {
      suppressTodoClickRef.current = true
      window.setTimeout(() => { suppressTodoClickRef.current = false }, 0)
    }
    setRevealedTodo(offset >= 37 ? start.id : null)
    setTodoDrag(null)
  }

  const cancelTodoGesture = () => {
    todoPointerStartRef.current = null
    setTodoDrag(null)
  }

  const consumeSuppressedTodoClick = () => {
    if (!suppressTodoClickRef.current) return false
    suppressTodoClickRef.current = false
    return true
  }

  const visibleTodos = useMemo(() => {
    const query = todoQuery.trim().toLocaleLowerCase('zh-CN')
    if (!query) return todoItems
    return todoItems.filter((todo) =>
      `${todo.title} ${todo.visual_summary}`.toLocaleLowerCase('zh-CN').includes(query),
    )
  }, [todoItems, todoQuery])

  const historyLibraryItems = useMemo(
    () =>
      historyItems
        .filter((item) => {
          const title = item.title.trim()
          return Boolean(
            item.preview.trim() ||
            item.is_pinned ||
            (title && title !== '新对话' && title !== '未命名对话'),
          )
        })
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
          hasCover: Boolean(item.cover),
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

  const historyVisibleIds = historyGroups.flatMap((group) => group.items.map((item) => item.id))
  const memoryVisibleIds = memoryGroups.flatMap((group) => group.items.map((item) => item.id))

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
        return action === 'archive'
          ? { ...item, is_pinned: false, archived_at: archivedAt }
          : { ...item, archived_at: null }
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
      setHistorySelectionMode(false)
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
        return action === 'archive'
          ? { ...item, is_pinned: false, archived_at: archivedAt }
          : { ...item, archived_at: null }
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
      setMemorySelectionMode(false)
      setRevealedItem(null)
    } catch (error) {
      setMemoryItems(previous)
      setMemoryError(error instanceof Error ? error.message : '操作失败，请重试')
    }
  }

  const confirmDelete = async () => {
    if (!deleteRequest) return
    const { kind, ids } = deleteRequest
    const setError = kind === 'history' ? setHistoryError : kind === 'memory' ? setMemoryError : setTodoError
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
        setHistorySelectionMode(false)
        if (selectedConversation && ids.includes(selectedConversation.id)) {
          setSelectedConversation(null)
          setHistoryMessages([])
        }
        const conversationOwner = conversationOwnershipRef.current.current()
        if (
          conversationOwner.conversationId &&
          ids.includes(conversationOwner.conversationId)
        ) {
          conversationOwnershipRef.current.release(conversationOwner.owner)
          setActiveConversationIdState(null)
        }
      } else if (kind === 'memory') {
        if (ids.length === 1) {
          await memoryMutation(server, accessToken, ids[0], 'delete')
        } else {
          await batchMemories(server, accessToken, ids, 'delete')
        }
        setMemoryItems((items) => items.filter((item) => !ids.includes(item.id)))
        setMemorySelection(new Set())
        setMemorySelectionMode(false)
        if (selectedMemoryId && ids.includes(selectedMemoryId)) setSelectedMemoryId(null)
      } else {
        await deleteTodo(server, accessToken, ids[0])
        setTodoItems((items) => items.filter((item) => item.id !== ids[0]))
        setRevealedTodo(null)
      }
      setRevealedItem(null)
      setDeleteRequest(null)
    } catch (error) {
      setError(error instanceof Error ? error.message : '删除失败，请重试')
      setDeleteRequest(null)
    }
  }

  const beginRenameConversation = (conversation: ConversationSummary) => {
    setRenameRequest(conversation)
    setRenameDraft(conversation.title || '')
    setRenameError('')
    setRevealedItem(null)
  }

  const beginRenameMemory = (memory: VisualMemory) => {
    setSelectedMemoryId(memory.id)
    setEditingMemoryId(memory.id)
    setMemoryDraft(memory.user_note || '')
    setRevealedItem(null)
  }

  const confirmRenameConversation = async () => {
    if (!renameRequest || !renameDraft.trim()) return
    setRenameBusy(true)
    setRenameError('')
    try {
      const updated = await renameConversation(
        server,
        accessToken,
        renameRequest.id,
        renameDraft.trim(),
      )
      setHistoryItems((items) => items.map((item) => (item.id === updated.id ? updated : item)))
      setSelectedConversation((item) => (item?.id === updated.id ? updated : item))
      setRenameRequest(null)
    } catch (error) {
      setRenameError(error instanceof Error ? error.message : '重命名失败，请重试')
    } finally {
      setRenameBusy(false)
    }
  }

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
              <span>让每一次开口，都有回响</span>
            </div>
          </header>

          <div className="auth-content">
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
          </div>
        </section>
      </main>
    )
  }

  return (
    <main
      ref={appShellRef}
      className="app-shell"
      data-edge-swipe-enabled={edgeSwipeBackEnabled ? 'true' : undefined}
    >
      <span className="edge-swipe-back-indicator" aria-hidden="true">
        <ArrowLeft />
      </span>
      {screen === 'home' && (
        <ConversationHome
          accountLabel={user.email}
          onStartAudio={() => openCall('audio')}
          onStartVideo={() => openCall('video')}
          onOpenMenu={() => setNavigationOpen(true)}
        />
      )}

      {screen === 'history' && (
        <section className="history-screen history-library-screen">
          <header className="screen-header history-page-header library-sticky-header">
            <button
              className="icon-button"
              type="button"
              aria-label="返回首页"
              onClick={() => navigateTo('home')}
            >
              <ArrowLeft />
            </button>
            <h1>对话记录</h1>
            <button
              className="icon-button"
              type="button"
              aria-label="开始新对话"
              onClick={() => openCall('audio')}
            >
              <NotePencil />
            </button>
          </header>

          <div className="library-region" aria-label="搜索聊天历史">
            <LibraryToolbar
              kind="聊天历史"
              query={historyQuery}
              scope={historyScope}
              selectionCount={historySelection.size}
              selectionMode={historySelectionMode}
              itemCount={historyVisibleIds.length}
              onQueryChange={setHistoryQuery}
              onScopeChange={(scope) => {
                setHistoryScope(scope)
                setHistorySelection(new Set())
                setHistorySelectionMode(false)
                setRevealedItem(null)
              }}
              onBatchAction={(action) =>
                void optimisticHistoryAction([...historySelection], action)
              }
              onStartSelection={() => {
                setHistorySelection(new Set())
                setHistorySelectionMode(true)
                setRevealedItem(null)
              }}
              onSelectAll={() => setHistorySelection(new Set(historyVisibleIds))}
              onCancelSelection={() => {
                setHistorySelection(new Set())
                setHistorySelectionMode(false)
              }}
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
              <h2>{historyQuery ? '没有找到相关对话' : historyScope === 'archived' ? '还没有归档对话' : historyScope === 'pinned' ? '还没有置顶对话' : '还没有聊天记录'}</h2>
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
                          className={`library-swipe-shell has-rename ${revealedItem === item.id ? 'is-revealed' : ''}`}
                          onPointerDown={(event) =>
                            beginLibraryGesture(event, item.id, () =>
                              {
                                setHistorySelectionMode(true)
                                toggleSelection(setHistorySelection, item.id)
                              },
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
                              if (historySelectionMode) {
                                toggleSelection(setHistorySelection, item.id)
                                return
                              }
                              preloadedConversationIdRef.current = null
                              setSelectedConversation(item)
                              navigateTo('conversation')
                            }}
                          >
                            {historySelectionMode && (
                              <span className={`selection-check ${selected ? 'is-selected' : ''}`} aria-hidden="true" />
                            )}
                            <span className="library-row-copy">
                              <span className="library-row-title">
                                <strong>{item.title || '未命名对话'}</strong>
                                <time>{formatHistoryTime(item.updated_at)}</time>
                              </span>
                              <span className="library-row-preview">{item.preview || '这次对话还没有文本内容'}</span>
                            </span>
                          </button>
                          <LibraryActions
                            pinned={item.is_pinned}
                            archived={item.archived_at !== null}
                            onAction={(action) => void optimisticHistoryAction([item.id], action)}
                            onRename={() => beginRenameConversation(item)}
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
          <header className="screen-header history-page-header library-sticky-header todo-header">
            <button className="icon-button" type="button" aria-label="返回首页" onClick={() => navigateTo('home')}>
              <ArrowLeft />
            </button>
            <div className="todo-heading">
              <h1>待办</h1>
              <p>{todoView === 'active' ? `${visibleTodos.length} 项待处理` : `${visibleTodos.length} 项已完成`}</p>
            </div>
            <button
              className="icon-button todo-add-button"
              type="button"
              aria-label="新建待办"
              onClick={() => setTodoEditor({ title: '', dueAt: '' })}
            >
              <Plus />
            </button>
          </header>
          <div className="todo-toolbar">
            <div className="todo-view-switch" role="tablist" aria-label="待办状态">
              <button
                className={todoView === 'active' ? 'is-active' : ''}
                type="button"
                role="tab"
                aria-selected={todoView === 'active'}
                onClick={() => {
                  setRevealedTodo(null)
                  setTodoView('active')
                }}
              >
                进行中
              </button>
              <button
                className={todoView === 'completed' ? 'is-active' : ''}
                type="button"
                role="tab"
                aria-selected={todoView === 'completed'}
                onClick={() => {
                  setRevealedTodo(null)
                  setTodoView('completed')
                }}
              >
                已完成
              </button>
            </div>
            <label className="todo-search">
              <Search aria-hidden="true" />
              <input aria-label="搜索待办" value={todoQuery} onChange={(event) => setTodoQuery(event.target.value)} placeholder="搜索待办" />
            </label>
          </div>
          {todoBusy && <div className="history-skeleton" aria-label="正在加载"><span /><span /><span /></div>}
          {todoError && <div className="history-error">{todoError}</div>}
          {!todoBusy && !todoError && visibleTodos.length === 0 && (
            <div className="history-empty">
              <ListChecks />
              <h2>{todoQuery ? '没有匹配的待办' : todoView === 'completed' ? '还没有已完成的待办' : '没有待处理事项'}</h2>
              {todoQuery ? (
                <p>换一个关键词试试。</p>
              ) : todoView === 'completed' ? (
                <p>完成待办后，会在这里保留画面、摘要和完成时间。</p>
              ) : (
                <>
                  <p>点击右上角加号新建，或在视频通话时说“把这个做成待办”。</p>
                  <button type="button" onClick={() => setTodoEditor({ title: '', dueAt: '' })}>新建待办</button>
                </>
              )}
            </div>
          )}
          {!todoBusy && visibleTodos.length > 0 && (
            <div className="todo-list">
              {visibleTodos.map((todo) => (
                <article
                  id={`todo-action-${encodeURIComponent(todo.id)}`}
                  tabIndex={-1}
                  className={`todo-swipe-shell ${revealedTodo === todo.id ? 'is-revealed' : ''} ${todoDrag?.id === todo.id ? 'is-dragging' : ''}`}
                  key={todo.id}
                  style={todoDrag?.id === todo.id ? { '--todo-drag-offset': `${todoDrag.offset}px` } as React.CSSProperties : undefined}
                  onPointerDown={(event) => beginTodoGesture(event, todo.id)}
                  onPointerMove={moveTodoGesture}
                  onPointerUp={endTodoGesture}
                  onPointerCancel={cancelTodoGesture}
                >
                  <button className="todo-swipe-delete danger-action" type="button" aria-label={`删除：${todo.title}`} onClick={() => setDeleteRequest({ kind: 'todo', ids: [todo.id] })}>
                    <Trash aria-hidden="true" /> 删除
                  </button>
                  <div className={`todo-card todo-card-surface ${todoView === 'completed' ? 'is-completed' : ''}`}>
                    <button
                      className="todo-complete"
                      type="button"
                      aria-label={todoView === 'active' ? `完成：${todo.title}` : `恢复：${todo.title}`}
                      onClick={() => {
                        if (consumeSuppressedTodoClick()) return
                        void setTodoCompleted(todo)
                      }}
                    >
                      {todoView === 'active' ? <Circle /> : <ArrowCounterClockwise />}
                    </button>
                    <button
                      className="todo-copy"
                      type="button"
                      aria-label={`编辑：${todo.title}`}
                      onClick={() => {
                        if (consumeSuppressedTodoClick()) return
                        setTodoEditor({ todo, title: todo.title, dueAt: todoDateInputValue(todo.due_at) })
                      }}
                    >
                      <strong>{todo.title}</strong>
                      {todo.visual_summary && <p>{todoSummaryLabel(todo.visual_summary)}</p>}
                      <time className={todo.due_at && todo.due_at < Date.now() / 1000 && todoView === 'active' ? 'is-overdue' : ''}>
                        {todoView === 'completed' && todo.completed_at ? `完成：${formatHistoryTime(todo.completed_at)}` : todoDueLabel(todo.due_at)}
                      </time>
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>
      )}

      {todoEditor && (
        <div className="confirm-dialog-backdrop" role="presentation">
          <section className="todo-editor" role="dialog" aria-modal="true" aria-labelledby="todo-editor-title">
            <h2 id="todo-editor-title">{todoEditor.todo ? '编辑待办' : '新建待办'}</h2>
            <label>
              事项
              <input autoFocus value={todoEditor.title} maxLength={500} onChange={(event) => setTodoEditor({ ...todoEditor, title: event.target.value })} placeholder="例如：买咖啡豆" />
            </label>
            <label>
              提醒时间
              <input type="datetime-local" value={todoEditor.dueAt} onChange={(event) => setTodoEditor({ ...todoEditor, dueAt: event.target.value })} />
            </label>
            <div className="todo-editor-actions">
              {todoEditor.dueAt && <button type="button" onClick={() => setTodoEditor({ ...todoEditor, dueAt: '' })}>清除提醒</button>}
              <span />
              <button type="button" onClick={() => setTodoEditor(null)}>取消</button>
              <button className="primary-button" type="button" disabled={!todoEditor.title.trim()} onClick={() => void saveTodo()}>保存</button>
            </div>
          </section>
        </div>
      )}

      {screen === 'memories' && (
        <section className="history-screen memory-screen">
          <header className="screen-header history-page-header library-sticky-header">
            <button className="icon-button" type="button" aria-label="返回首页" onClick={() => navigateTo('home')}>
              <ArrowLeft />
            </button>
            <h1>记忆</h1>
            <span className="header-spacer" />
          </header>

          <div className="library-region" aria-label="搜索视觉记忆">
            <LibraryToolbar
              kind="记忆"
              query={memoryQuery}
              scope={memoryScope}
              selectionCount={memorySelection.size}
              selectionMode={memorySelectionMode}
              itemCount={memoryVisibleIds.length}
              onQueryChange={setMemoryQuery}
              onScopeChange={(scope) => {
                setMemoryScope(scope)
                setMemorySelection(new Set())
                setMemorySelectionMode(false)
                setRevealedItem(null)
              }}
              onBatchAction={(action) =>
                void optimisticMemoryAction([...memorySelection], action)
              }
              onStartSelection={() => {
                setMemorySelection(new Set())
                setMemorySelectionMode(true)
                setRevealedItem(null)
              }}
              onSelectAll={() => setMemorySelection(new Set(memoryVisibleIds))}
              onCancelSelection={() => {
                setMemorySelection(new Set())
                setMemorySelectionMode(false)
              }}
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
              <h2>{memoryQuery ? '没有找到相关记忆' : memoryScope === 'archived' ? '还没有归档记忆' : memoryScope === 'pinned' ? '还没有置顶记忆' : memoryScope === 'images' ? '还没有图片记忆' : '还没有保存记忆'}</h2>
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
                          className={`memory-card library-swipe-shell has-rename ${memory.cover ? 'has-cover' : 'is-text-memory'} ${revealedItem === memory.id ? 'is-revealed' : ''}`}
                          onPointerDown={(event) =>
                            beginLibraryGesture(event, memory.id, () =>
                              {
                                setMemorySelectionMode(true)
                                toggleSelection(setMemorySelection, memory.id)
                              },
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
                              if (memorySelectionMode) {
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
                            {memorySelectionMode && (
                              <span className={`selection-check card-check ${selected ? 'is-selected' : ''}`} aria-hidden="true" />
                            )}
                            {memory.is_pinned && <PushPin className="memory-pin" aria-label="已置顶" />}
                            <span className="memory-card-body">
                              <strong>{memoryDisplayTitle(memory)}</strong>
                              {memory.visual_summary && <span className="memory-card-note">{memory.visual_summary}</span>}
                              <time>{formatHistoryTime(memory.captured_at ?? memory.created_at)}</time>
                            </span>
                          </button>
                          <LibraryActions
                            pinned={memory.is_pinned}
                            archived={memory.archived_at !== null}
                            onAction={(action) => void optimisticMemoryAction([memory.id], action)}
                            onRename={() => beginRenameMemory(memory)}
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
                  <button type="button" aria-label="返回记忆列表" onClick={() => setSelectedMemoryId(null)}><ArrowLeft /></button>
                  <h2 id="memory-detail-title">记忆详情</h2>
                  <span className="header-spacer" aria-hidden="true" />
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
                      <button type="button" disabled={!memoryDraft.trim()} onClick={() => void saveMemoryEdit(selectedMemory.id)}>保存</button>
                      <button type="button" onClick={() => { setEditingMemoryId(null); setMemoryDraft('') }}>取消</button>
                    </>
                  ) : (
                    <button type="button" onClick={() => beginRenameMemory(selectedMemory)}><NotePencil /> 编辑名称</button>
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
              onClick={() => navigateTo('history')}
            >
              <ArrowLeft />
            </button>
            <h1 className="conversation-header-title">{selectedConversation.title || '未命名对话'}</h1>
            <button
              className="icon-button"
              type="button"
              aria-label="重命名对话"
              onClick={() => beginRenameConversation(selectedConversation)}
            >
              <DotsThreeVertical aria-hidden="true" />
            </button>
          </header>

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
                  <div className="message-meta">
                    <strong>{message.role === 'user' ? '你' : 'Ripple'}</strong>
                    <time>{formatHistoryTime(message.created_at)}</time>
                  </div>
                  <MarkdownContent>{message.content}</MarkdownContent>
                  <ConversationActions
                    actions={message.actions}
                    onActivate={openConversationAction}
                  />
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
          <aside className="conversation-continuation-bar" aria-label="继续这段对话">
            <button
              className="continuation-video"
              type="button"
              aria-label="用视频继续"
              onClick={() => openCall('video', selectedConversation.id)}
            >
              <Video aria-hidden="true" />
            </button>
            <button
              className="continuation-compose"
              type="button"
              onClick={() => openCall('audio', selectedConversation.id)}
            >
              <Microphone aria-hidden="true" />
              <span>继续语音对话</span>
            </button>
          </aside>
        </section>
      )}

      {screen === 'settings' && (
        <section className="settings-screen profile-screen">
          <header className="screen-header">
            <button className="icon-button" type="button" aria-label="返回首页" onClick={() => navigateTo('home')}>
              <ArrowLeft />
            </button>
            <h1>设置</h1>
            <span className="header-spacer" />
          </header>

          <div className="profile-groups">
            <div className="profile-identity">
              <button
                className="profile-avatar-button"
                type="button"
                aria-label="选择并更换头像"
                onClick={() => avatarInputRef.current?.click()}
              >
                <UserAvatar
                  server={server}
                  token={accessToken}
                  email={user.email}
                  avatarUrl={user.avatar_url}
                />
                <span className="profile-avatar-edit" aria-hidden="true"><ImagesSquare /></span>
              </button>
              <input
                ref={avatarInputRef}
                className="avatar-file-input"
                type="file"
                accept="image/jpeg,image/png,image/webp"
                aria-label="选择头像图片"
                onChange={selectAvatarFile}
              />
              <div>
                <strong>{user.email}</strong>
                <small>点击头像更换</small>
              </div>
            </div>
            {avatarNotice ? <p className="avatar-notice" role="status">{avatarNotice}</p> : null}
            <section className="profile-section" aria-labelledby="profile-account-heading">
              <h2 id="profile-account-heading">系统状态</h2>
              <dl className="profile-info-list">
                <div className="profile-info-row">
                  <dt>通知权限</dt>
                  <dd>{notificationPermissionLabel()}</dd>
                </div>
              </dl>
            </section>

            <section className="profile-section" aria-labelledby="profile-experience-heading">
              <h2 id="profile-experience-heading">通话体验</h2>
              <div className="profile-copy-row">
                <strong>实时字幕</strong>
                <p>通话时自动显示你和 Ripple 正在说的内容。</p>
              </div>
              <button className="profile-navigation-row" type="button" onClick={() => navigateTo('memories')}>
                <span>
                  <strong>视觉记忆</strong>
                  <small>查看通话中保存的画面与备注</small>
                </span>
                <ChevronRight aria-hidden="true" />
              </button>
            </section>

            <section className="profile-section" aria-labelledby="profile-privacy-heading">
              <h2 id="profile-privacy-heading">数据使用</h2>
              <div className="profile-copy-row">
                <strong>麦克风与相机</strong>
                <p>麦克风音频仅用于实时对话；相机画面只在你开启视频或 Ripple 请求画面时发送到已连接服务。</p>
              </div>
            </section>

            <button className="profile-logout" type="button" onClick={() => void signOut()}>
              <SignOut aria-hidden="true" />
              退出登录
            </button>
          </div>
        </section>
      )}

      {screen === 'personalization' && (
        <section className="settings-screen profile-screen personalization-screen">
          <header className="screen-header">
            <button className="icon-button" type="button" aria-label="返回首页" onClick={() => navigateTo('home')}>
              <ArrowLeft />
            </button>
            <h1>个性化</h1>
            <span className="header-spacer" />
          </header>
          <div className="profile-groups">
            <PersonalizationSection server={server} token={accessToken} />
          </div>
        </section>
      )}

      {screen === 'meetings' && (
        <MeetingRecords
          items={meetingItems}
          detail={selectedMeeting}
          busy={meetingBusy}
          error={meetingError}
          onBack={() => navigateTo('home')}
          onOpen={(id) => void openMeetingRecord(id)}
          onCloseDetail={() => setSelectedMeeting(null)}
          onRetry={(id) => void retryMeetingGeneration(id)}
          onPromoteAction={(meetingId, actionId) => void addMeetingActionToTodos(meetingId, actionId)}
        />
      )}

      {screen === 'projects' && (
        <SecondaryScaffold title="项目" icon={FolderKanban} onBack={() => navigateTo('home')} />
      )}

      {screen === 'materials' && (
        <SecondaryScaffold title="资料库" icon={Database} onBack={() => navigateTo('home')} />
      )}

      {screen === 'call' && (
        <LiveCallScreen
          mode={mode}
          cameraPhase={cameraPhase}
          cameraPreviewVisible={cameraPreviewVisible}
          cameraControlReady={cameraControlReady}
          frameRequestActive={frameRequestActive}
          state={sessionState}
          elapsed={elapsed}
          muted={muted}
          inputLevel={inputLevel}
          outputLevel={outputLevel}
          rippleSignals={rippleSignals}
          onRippleSignalsConsumed={onRippleSignalsConsumed}
          userText={userText}
          assistantText={assistantText}
          toolStatus={toolStatus}
          errorMessage={visibleCallError(errorMessage, cameraErrorMessage)}
          artifacts={liveArtifacts}
          results={liveResults}
          transcript={liveTranscript}
          transcriptError={meetingCaptureError}
          server={server}
          accessToken={accessToken}
          videoRef={videoRef}
          captureCanvasRef={canvasRef}
          onToggleMute={toggleMute}
          onToggleCamera={toggleCamera}
          onFlipCamera={flipCamera}
          onDismissResult={(callId) =>
            dispatchLiveResults({ type: 'dismiss', callId })
          }
          onLeave={leaveCall}
        />
      )}

      {screen !== 'call' && (
        <AppDrawer
          open={navigationOpen}
          active={destinationForScreen(screen)}
          accountLabel={user.email}
          avatarUrl={user.avatar_url}
          server={server}
          token={accessToken}
          onClose={() => setNavigationOpen(false)}
          onSelect={selectDestination}
        />
      )}

      {avatarFile ? (
        <AvatarEditor
          file={avatarFile}
          busy={avatarBusy}
          error={avatarError}
          onCancel={() => {
            setAvatarFile(null)
            setAvatarError('')
          }}
          onSave={saveAvatar}
        />
      ) : null}

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
              删除{deleteRequest.ids.length > 1
                ? `${deleteRequest.ids.length} 项`
                : deleteRequest.kind === 'history'
                  ? '这段对话'
                  : deleteRequest.kind === 'todo' ? '这条待办' : '这条记忆'}？
            </h2>
            <p id="delete-dialog-description">
              {deleteRequest.kind === 'todo'
                ? '删除后无法恢复，提醒和完成记录也会一并移除。'
                : deleteRequest.kind === 'history'
                  ? '删除后无法恢复，但视觉记忆和待办会分别保留。'
                  : '删除后无法恢复，但不会删除关联的聊天记录。'}
            </p>
            <div>
              <button type="button" onClick={() => setDeleteRequest(null)}>取消</button>
              <button className="danger-action" type="button" autoFocus onClick={() => void confirmDelete()}>确认删除</button>
            </div>
          </section>
        </div>
      )}

      {renameRequest && (
        <div className="confirm-dialog-backdrop" role="presentation">
          <section
            className="confirm-dialog rename-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rename-dialog-title"
          >
            <span className="confirm-dialog-mark"><NotePencil aria-hidden="true" /></span>
            <h2 id="rename-dialog-title">重命名对话</h2>
            <label className="visually-hidden" htmlFor="conversation-title-input">对话名称</label>
            <input
              id="conversation-title-input"
              value={renameDraft}
              maxLength={80}
              autoFocus
              onChange={(event) => setRenameDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void confirmRenameConversation()
              }}
            />
            {renameError && <p className="form-error">{renameError}</p>}
            <div>
              <button type="button" disabled={renameBusy} onClick={() => setRenameRequest(null)}>取消</button>
              <button
                className="primary-action"
                type="button"
                disabled={renameBusy || !renameDraft.trim()}
                onClick={() => void confirmRenameConversation()}
              >
                {renameBusy ? '正在保存' : '保存'}
              </button>
            </div>
          </section>
        </div>
      )}
    </main>
  )
}
