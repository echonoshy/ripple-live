import type { LibraryAction, LibraryListOptions } from './library'

export type AuthUser = {
  id: string
  email: string
}

export type AuthSession = {
  access_token: string
  token_type: 'bearer'
  user: AuthUser
}

export type ConversationSummary = {
  id: string
  title: string
  preview: string
  created_at: number
  updated_at: number
  is_pinned: boolean
  archived_at: number | null
}

export type ConversationMessage = {
  id: number
  role: 'user' | 'assistant' | string
  content: string
  created_at: number
  attachments: MemoryArtifact[]
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
  if (init.body) headers.set('Content-Type', 'application/json')
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

export async function conversationMessages(
  server: string,
  token: string,
  conversationId: string,
) {
  const payload = await request<{ data: ConversationMessage[] }>(
    server,
    `/v1/conversations/${encodeURIComponent(conversationId)}/messages?limit=500`,
    {},
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

export async function assetBlob(
  server: string,
  token: string,
  contentUrl: string,
) {
  const response = await fetch(`${httpBase(server)}${contentUrl}`, {
    headers: { Authorization: `Bearer ${token}` },
  })
  if (!response.ok) throw new Error(`图片加载失败 (${response.status})`)
  return response.blob()
}
