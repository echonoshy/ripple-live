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

export async function conversations(server: string, token: string) {
  const payload = await request<{ data: ConversationSummary[] }>(
    server,
    '/v1/conversations?limit=50',
    {},
    token,
  )
  return payload.data
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

export async function memories(server: string, token: string) {
  const payload = await request<{ data: VisualMemory[] }>(
    server,
    '/v1/memories?limit=100',
    {},
    token,
  )
  return payload.data
}

export async function updateMemory(
  server: string,
  token: string,
  memoryId: string,
  userNote: string,
) {
  const payload = await request<{ data: VisualMemory }>(
    server,
    `/v1/memories/${encodeURIComponent(memoryId)}`,
    { method: 'PATCH', body: JSON.stringify({ user_note: userNote }) },
    token,
  )
  return payload.data
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
