import type { LibraryAction, LibraryListOptions } from './library'

export type AuthUser = {
  id: string
  email: string
  avatar_url: string | null
}

export type AuthSession = {
  access_token: string
  token_type: 'bearer'
  user: AuthUser
}

export type UserProfile = {
  ai_identity: string
  user_identity: string
  preferred_name: string
  basic_memory: string
  updated_at: number | null
}

export type UserProfileUpdate = Omit<UserProfile, 'updated_at'>

export type ConversationSummary = {
  id: string
  title: string
  preview: string
  created_at: number
  updated_at: number
  is_pinned: boolean
  archived_at: number | null
}

export type ProjectRecord = {
  id: string
  name: string
  description: string
  instructions: string
  created_at: number
  updated_at: number
  archived_at: number | null
}

export type ProjectCreate = {
  name: string
  description?: string
  instructions?: string
}

export type ProjectPatch = {
  name?: string
  description?: string
  instructions?: string
  archived?: boolean
}

export type ConversationMessage = {
  id: number
  role: 'user' | 'assistant' | string
  content: string
  created_at: number
  attachments: MemoryArtifact[]
  actions: ConversationAction[]
}

export type ConversationAction = {
  kind: 'memory' | 'todo' | string
  target_id: string
  label: string
  due_at: number | null
}

export type MemoryArtifact = {
  id: string
  kind: 'image' | string
  memory_id?: string
  caption: string
  content_url: string
}

export type VisualMemory = {
  id: string
  kind: 'visual' | 'text' | string
  user_note: string
  visual_summary: string
  captured_at?: number | null
  created_at: number
  cover?: MemoryArtifact | null
  is_pinned: boolean
  archived_at: number | null
}

export type MemoryFactKind =
  | 'identity'
  | 'preference'
  | 'relationship'
  | 'habit'
  | 'context'
  | 'other'

export type MemoryFact = {
  id: string
  kind: MemoryFactKind
  summary: string
  scope_type: 'personal' | 'project'
  project_id: string | null
  source: 'manual' | 'conversation'
  created_at: number
  updated_at: number
}

export type MemoryFactPatch = {
  kind?: MemoryFactKind
  summary?: string
}

export type TodoItem = {
  id: string
  memory_id: string | null
  title: string
  visual_summary: string
  due_at: number | null
  completed_at: number | null
  created_at: number
  cover: MemoryArtifact | null
}

export type TodoPatch = {
  title?: string
  due_at?: number | null
  clear_due_at?: boolean
  completed?: boolean
}

export type MeetingRecord = {
  id: string
  conversation_id: string | null
  mode: 'audio' | 'video'
  status: 'recording' | 'processing' | 'ready' | 'failed'
  started_at: number
  ended_at: number | null
  duration_seconds: number | null
  title: string
  summary: string
  generated_at: number | null
  last_error: string | null
  transcript_count: number
  action_count: number
}

export type MeetingTranscriptSegment = {
  id: number
  role: 'user' | 'assistant'
  content: string
  created_at: number
  ordinal: number
}

export type MeetingActionItem = {
  id: string
  title: string
  due_at: number | null
  todo_id: string | null
  ordinal: number
}

export type MeetingDetail = MeetingRecord & {
  transcript: MeetingTranscriptSegment[]
  action_items: MeetingActionItem[]
}

export type LibraryPatch = {
  title?: string
  is_pinned?: boolean
  archived?: boolean
}

export type MemoryPatch = LibraryPatch & {
  user_note?: string
}

function httpBase(server: string) {
  return `http://${server
    .trim()
    .replace(/^https?:\/\//, '')
    .replace(/^wss?:\/\//, '')
    .replace(/\/+$/, '')}`
}

async function request<T>(
  server: string,
  path: string,
  init: RequestInit = {},
  token?: string,
): Promise<T> {
  const headers = new Headers(init.headers)
  headers.set('Accept', 'application/json')
  if (typeof init.body === 'string' && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  if (token) headers.set('Authorization', `Bearer ${token}`)
  const response = await fetch(`${httpBase(server)}${path}`, {
    ...init,
    headers,
  })
  if (!response.ok) {
    const payload = (await response.json().catch(() => null)) as {
      error?: { message?: string }
    } | null
    throw new Error(payload?.error?.message ?? `请求失败 (${response.status})`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export function login(server: string, email: string, password: string) {
  return request<AuthSession>(server, '/v1/auth/login', {
    method: 'POST',
    body: JSON.stringify({ email, password }),
  })
}

export function register(
  server: string,
  email: string,
  password: string,
  invitationCode: string,
) {
  return request<AuthSession>(server, '/v1/auth/register', {
    method: 'POST',
    body: JSON.stringify({
      email,
      password,
      invitation_code: invitationCode,
    }),
  })
}

export async function currentUser(server: string, token: string) {
  const payload = await request<{ user: AuthUser }>(
    server,
    '/v1/auth/me',
    {},
    token,
  )
  return payload.user
}

export function logout(server: string, token: string) {
  return request<void>(server, '/v1/auth/logout', { method: 'POST' }, token)
}

export async function userProfile(server: string, token: string) {
  const payload = await request<{ data: UserProfile }>(
    server,
    '/v1/profile',
    {},
    token,
  )
  return payload.data
}

export async function updateUserProfile(
  server: string,
  token: string,
  profile: UserProfileUpdate,
) {
  const payload = await request<{ data: UserProfile }>(
    server,
    '/v1/profile',
    { method: 'PUT', body: JSON.stringify(profile) },
    token,
  )
  return payload.data
}

export async function uploadUserAvatar(server: string, token: string, avatar: Blob) {
  const payload = await request<{ user: AuthUser }>(
    server,
    '/v1/auth/me/avatar',
    {
      method: 'PUT',
      headers: { 'Content-Type': 'image/jpeg' },
      body: avatar,
    },
    token,
  )
  return payload.user
}

export async function clearUserAvatar(server: string, token: string) {
  const payload = await request<{ user: AuthUser }>(
    server,
    '/v1/auth/me/avatar',
    { method: 'DELETE' },
    token,
  )
  return payload.user
}

function librarySearchParams(options: LibraryListOptions) {
  const params = new URLSearchParams({
    scope: options.scope,
    query: options.query,
    limit: String(options.limit),
  })
  if (options.pinned !== undefined) params.set('pinned', String(options.pinned))
  return params
}

export async function conversations(
  server: string,
  token: string,
  options: LibraryListOptions = {
    scope: 'active',
    query: '',
    limit: 50,
  },
) {
  const payload = await request<{ data: ConversationSummary[] }>(
    server,
    `/v1/conversations?${librarySearchParams(options)}`,
    {},
    token,
  )
  return payload.data
}

export async function projects(
  server: string,
  token: string,
  options: LibraryListOptions = {
    scope: 'active',
    query: '',
    limit: 50,
  },
) {
  const payload = await request<{ data: ProjectRecord[] }>(
    server,
    `/v1/projects?${librarySearchParams(options)}`,
    {},
    token,
  )
  return payload.data
}

export async function project(server: string, token: string, id: string) {
  const payload = await request<{ data: ProjectRecord }>(
    server,
    `/v1/projects/${encodeURIComponent(id)}`,
    {},
    token,
  )
  return payload.data
}

export async function createProject(
  server: string,
  token: string,
  input: ProjectCreate,
) {
  const payload = await request<{ data: ProjectRecord }>(
    server,
    '/v1/projects',
    { method: 'POST', body: JSON.stringify(input) },
    token,
  )
  return payload.data
}

export async function updateProject(
  server: string,
  token: string,
  id: string,
  patch: ProjectPatch,
) {
  const payload = await request<{ data: ProjectRecord }>(
    server,
    `/v1/projects/${encodeURIComponent(id)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
    token,
  )
  return payload.data
}

export function archiveProject(server: string, token: string, id: string) {
  return request<void>(
    server,
    `/v1/projects/${encodeURIComponent(id)}`,
    { method: 'DELETE' },
    token,
  )
}

export async function projectConversations(
  server: string,
  token: string,
  projectId: string,
  options: LibraryListOptions = {
    scope: 'active',
    query: '',
    limit: 50,
  },
) {
  const payload = await request<{ data: ConversationSummary[] }>(
    server,
    `/v1/projects/${encodeURIComponent(projectId)}/conversations?${librarySearchParams(options)}`,
    {},
    token,
  )
  return payload.data
}

export async function createProjectConversation(
  server: string,
  token: string,
  projectId: string,
) {
  return request<{ id: string; project_id: string }>(
    server,
    `/v1/projects/${encodeURIComponent(projectId)}/conversations`,
    { method: 'POST' },
    token,
  )
}

export async function conversation(
  server: string,
  token: string,
  id: string,
) {
  const payload = await request<{ data: ConversationSummary }>(
    server,
    `/v1/conversations/${encodeURIComponent(id)}`,
    {},
    token,
  )
  return payload.data
}

export async function updateConversation(
  server: string,
  token: string,
  id: string,
  patch: LibraryPatch,
) {
  const payload = await request<{ data: ConversationSummary }>(
    server,
    `/v1/conversations/${encodeURIComponent(id)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
    token,
  )
  return payload.data
}

export function deleteConversation(server: string, token: string, id: string) {
  return request<void>(
    server,
    `/v1/conversations/${encodeURIComponent(id)}`,
    { method: 'DELETE' },
    token,
  )
}

export function conversationMutation(
  server: string,
  token: string,
  id: string,
  action: LibraryAction,
) {
  if (action === 'delete') return deleteConversation(server, token, id)
  if (action === 'pin' || action === 'unpin') {
    return updateConversation(server, token, id, {
      is_pinned: action === 'pin',
    })
  }
  return updateConversation(server, token, id, {
    archived: action === 'archive',
  })
}

export function renameConversation(
  server: string,
  token: string,
  id: string,
  title: string,
) {
  return updateConversation(server, token, id, { title })
}

async function batchMutation(
  server: string,
  token: string,
  path: string,
  ids: string[],
  action: LibraryAction,
) {
  return request<{ updated: number }>(
    server,
    path,
    { method: 'POST', body: JSON.stringify({ ids, action }) },
    token,
  )
}

export function batchConversations(
  server: string,
  token: string,
  ids: string[],
  action: LibraryAction,
) {
  return batchMutation(server, token, '/v1/conversations/batch', ids, action)
}

function ownDataValue(value: unknown, key: string) {
  if (!value || typeof value !== 'object') return undefined
  try {
    const descriptor = Object.getOwnPropertyDescriptor(value, key)
    return descriptor && 'value' in descriptor ? descriptor.value : undefined
  } catch {
    return undefined
  }
}

function safeArrayValues(value: unknown, limit: number): unknown[] {
  try {
    if (!Array.isArray(value)) return []
  } catch {
    return []
  }
  const rawLength = ownDataValue(value, 'length')
  if (typeof rawLength !== 'number' || !Number.isSafeInteger(rawLength) || rawLength < 0) {
    return []
  }
  const values: unknown[] = []
  for (let index = 0; index < Math.min(rawLength, limit); index += 1) {
    try {
      const descriptor = Object.getOwnPropertyDescriptor(value, String(index))
      if (descriptor && 'value' in descriptor) values.push(descriptor.value)
    } catch {
      // A malformed row must not discard other rows from the same response.
    }
  }
  return values
}

export function normalizeConversationMessages(
  messages: unknown,
): ConversationMessage[] {
  return safeArrayValues(messages, 500).flatMap((message): ConversationMessage[] => {
    const id = ownDataValue(message, 'id')
    const role = ownDataValue(message, 'role')
    const content = ownDataValue(message, 'content')
    const createdAt = ownDataValue(message, 'created_at')
    if (
      typeof id !== 'number' || !Number.isFinite(id) ||
      typeof role !== 'string' ||
      typeof content !== 'string' ||
      typeof createdAt !== 'number' || !Number.isFinite(createdAt)
    ) return []

    const actions = safeArrayValues(ownDataValue(message, 'actions'), 10)
      .flatMap((candidate): ConversationAction[] => {
        const kind = ownDataValue(candidate, 'kind')
        const targetId = ownDataValue(candidate, 'target_id')
        const label = ownDataValue(candidate, 'label')
        const dueAt = ownDataValue(candidate, 'due_at')
        if (
          typeof kind !== 'string' ||
          typeof targetId !== 'string' ||
          typeof label !== 'string'
        ) return []
        return [{
          kind,
          target_id: targetId,
          label,
          due_at: typeof dueAt === 'number' && Number.isFinite(dueAt)
            ? dueAt
            : null,
        }]
      })

    return [{
      id,
      role,
      content,
      created_at: createdAt,
      attachments: safeArrayValues(
        ownDataValue(message, 'attachments'),
        100,
      ) as MemoryArtifact[],
      actions,
    }]
  })
}

export async function conversationMessages(
  server: string,
  token: string,
  conversationId: string,
) {
  const payload = await request<{ data: unknown }>(
    server,
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages?limit=500`,
    {},
    token,
  )
  return normalizeConversationMessages(payload.data)
}

export async function meetings(server: string, token: string, limit = 50) {
  const payload = await request<{ data: MeetingRecord[] }>(
    server,
    `/v1/meetings?limit=${Math.max(1, Math.min(100, limit))}`,
    {},
    token,
  )
  return payload.data
}

export async function meeting(server: string, token: string, id: string) {
  const payload = await request<{ data: MeetingDetail }>(
    server,
    `/v1/meetings/${encodeURIComponent(id)}`,
    {},
    token,
  )
  return payload.data
}

export async function startMeeting(
  server: string,
  token: string,
  conversationId: string,
  mode: 'audio' | 'video',
) {
  const payload = await request<{ data: MeetingRecord }>(
    server,
    '/v1/meetings',
    {
      method: 'POST',
      body: JSON.stringify({ conversation_id: conversationId, mode }),
    },
    token,
  )
  return payload.data
}

export async function finishMeeting(server: string, token: string, id: string) {
  const payload = await request<{ data: MeetingRecord }>(
    server,
    `/v1/meetings/${encodeURIComponent(id)}/finish`,
    { method: 'POST' },
    token,
  )
  return payload.data
}

export async function regenerateMeeting(server: string, token: string, id: string) {
  const payload = await request<{ data: MeetingRecord }>(
    server,
    `/v1/meetings/${encodeURIComponent(id)}/generate`,
    { method: 'POST' },
    token,
  )
  return payload.data
}

export async function promoteMeetingAction(
  server: string,
  token: string,
  meetingId: string,
  actionId: string,
) {
  const payload = await request<{ data: MeetingActionItem }>(
    server,
    `/v1/meetings/${encodeURIComponent(meetingId)}/actions/${encodeURIComponent(actionId)}/todo`,
    { method: 'POST' },
    token,
  )
  return payload.data
}

export async function memories(
  server: string,
  token: string,
  options: LibraryListOptions = {
    scope: 'active',
    query: '',
    limit: 100,
  },
) {
  const payload = await request<{ data: VisualMemory[] }>(
    server,
    `/v1/memories?${librarySearchParams(options)}`,
    {},
    token,
  )
  return payload.data
}

export async function memory(
  server: string,
  token: string,
  memoryId: string,
) {
  const payload = await request<{ data: VisualMemory }>(
    server,
    `/v1/memories/${encodeURIComponent(memoryId)}`,
    {},
    token,
  )
  return payload.data
}

export async function todos(
  server: string,
  token: string,
  completed = false,
) {
  const payload = await request<{ data: TodoItem[] }>(
    server,
    `/v1/todos?completed=${completed ? 'true' : 'false'}&limit=100`,
    {},
    token,
  )
  return payload.data
}

export async function updateTodo(
  server: string,
  token: string,
  todoId: string,
  patch: TodoPatch,
) {
  const payload = await request<{ data: TodoItem }>(
    server,
    `/v1/todos/${encodeURIComponent(todoId)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
    token,
  )
  return payload.data
}

export async function createTodo(
  server: string,
  token: string,
  title: string,
  dueAt?: number,
) {
  const payload = await request<{ data: TodoItem }>(
    server,
    '/v1/todos',
    {
      method: 'POST',
      body: JSON.stringify({ title, due_at: dueAt }),
    },
    token,
  )
  return payload.data
}

export function deleteTodo(server: string, token: string, todoId: string) {
  return request<void>(
    server,
    `/v1/todos/${encodeURIComponent(todoId)}`,
    { method: 'DELETE' },
    token,
  )
}

export async function updateMemory(
  server: string,
  token: string,
  memoryId: string,
  patch: string | MemoryPatch,
) {
  const payload = await request<{ data: VisualMemory }>(
    server,
    `/v1/memories/${encodeURIComponent(memoryId)}`,
    {
      method: 'PATCH',
      body: JSON.stringify(
        typeof patch === 'string' ? { user_note: patch } : patch,
      ),
    },
    token,
  )
  return payload.data
}

export function memoryMutation(
  server: string,
  token: string,
  id: string,
  action: LibraryAction,
) {
  if (action === 'delete') return deleteMemory(server, token, id)
  if (action === 'pin' || action === 'unpin') {
    return updateMemory(server, token, id, { is_pinned: action === 'pin' })
  }
  return updateMemory(server, token, id, {
    archived: action === 'archive',
  })
}

export function batchMemories(
  server: string,
  token: string,
  ids: string[],
  action: LibraryAction,
) {
  return batchMutation(server, token, '/v1/memories/batch', ids, action)
}

export function deleteMemory(
  server: string,
  token: string,
  memoryId: string,
) {
  return request<void>(
    server,
    `/v1/memories/${encodeURIComponent(memoryId)}`,
    { method: 'DELETE' },
    token,
  )
}

export async function memoryFacts(
  server: string,
  token: string,
  query = '',
) {
  const params = new URLSearchParams({ query, limit: '100' })
  const payload = await request<{ data: MemoryFact[] }>(
    server,
    `/v1/memory-facts?${params}`,
    {},
    token,
  )
  return payload.data
}

export async function createMemoryFact(
  server: string,
  token: string,
  kind: MemoryFactKind,
  summary: string,
) {
  const payload = await request<{ data: MemoryFact }>(
    server,
    '/v1/memory-facts',
    { method: 'POST', body: JSON.stringify({ kind, summary }) },
    token,
  )
  return payload.data
}

export async function updateMemoryFact(
  server: string,
  token: string,
  factId: string,
  patch: MemoryFactPatch,
) {
  const payload = await request<{ data: MemoryFact }>(
    server,
    `/v1/memory-facts/${encodeURIComponent(factId)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
    token,
  )
  return payload.data
}

export function deleteMemoryFact(
  server: string,
  token: string,
  factId: string,
) {
  return request<void>(
    server,
    `/v1/memory-facts/${encodeURIComponent(factId)}`,
    { method: 'DELETE' },
    token,
  )
}

export async function assetBlob(
  server: string,
  token: string,
  contentUrl: string,
  signal?: AbortSignal,
) {
  const response = await fetch(`${httpBase(server)}${contentUrl}`, {
    headers: { Authorization: `Bearer ${token}` },
    signal,
  })
  if (!response.ok) throw new Error(`图片加载失败 (${response.status})`)
  return response.blob()
}
