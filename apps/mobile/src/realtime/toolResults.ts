export type LiveResult =
  | {
      kind: 'memory_receipt'
      callId: string
      memoryId: string
      title: string
      status: 'success'
    }
  | {
      kind: 'todo_receipt'
      callId: string
      todoId: string
      title: string
      dueAt: number | null
      status: 'success'
    }
  | {
      kind: 'todo_list'
      callId: string
      titles: string[]
      completed: boolean
      status: 'success'
    }
  | {
      kind: 'search'
      callId: string
      items: Array<{ title: string; url: string; snippet: string }>
      status: 'success'
    }
  | {
      kind: 'weather'
      callId: string
      location: string
      summary: string
      temperature: number | null
      status: 'success'
    }
  | {
      kind: 'generic'
      callId: string
      label: string
      status: 'success' | 'error'
    }

export type ToolCompletion = { callId: string; name: string; result: unknown }

const MAX_LABEL_LENGTH = 120
const MAX_TEXT_LENGTH = 120
const MAX_SNIPPET_LENGTH = 240
const MAX_URL_LENGTH = 2048
const MAX_TODO_ROWS = 5
const MAX_SEARCH_ROWS = 3
const INVALID_CALL_ID = 'unknown-call'
const ZERO_WIDTH_JOINER = '\u200D'
const graphemeExtension = /^(?:\p{Mark}|\p{Emoji_Modifier}|\p{Variation_Selector})$/u

const segmenter =
  typeof Intl !== 'undefined' && typeof Intl.Segmenter === 'function'
    ? new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    : null

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function fallbackGraphemes(value: string) {
  const graphemes: string[] = []
  let current = ''
  let joined = false
  for (const character of value) {
    if (!current) {
      current = character
      continue
    }
    if (joined || character === ZERO_WIDTH_JOINER || graphemeExtension.test(character)) {
      current += character
      joined = character === ZERO_WIDTH_JOINER
      continue
    }
    graphemes.push(current)
    current = character
  }
  if (current) graphemes.push(current)
  return graphemes
}

function truncate(value: string, maximum: number) {
  if (segmenter) {
    const graphemes: string[] = []
    for (const item of segmenter.segment(value)) {
      if (graphemes.length === maximum) break
      graphemes.push(item.segment)
    }
    return graphemes.join('')
  }
  return fallbackGraphemes(value).slice(0, maximum).join('')
}

function displayText(value: unknown, maximum = MAX_TEXT_LENGTH): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  return trimmed ? truncate(trimmed, maximum) : null
}

function hasControlCharacter(value: string) {
  for (const character of value) {
    const codePoint = character.codePointAt(0)
    if (codePoint !== undefined && (codePoint <= 0x1f || codePoint === 0x7f)) return true
  }
  return false
}

function identifier(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (!trimmed || hasControlCharacter(value)) return null
  return value
}

function finiteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function sourceUrl(value: unknown): string | null {
  if (typeof value !== 'string' || !value || value !== value.trim()) return null
  if (Array.from(value).length > MAX_URL_LENGTH || /\s/.test(value)) return null
  if (!/^https?:\/\//i.test(value)) return null

  try {
    const parsed = new URL(value)
    if (
      (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') ||
      !parsed.hostname ||
      parsed.username ||
      parsed.password
    ) {
      return null
    }
    return parsed.href
  } catch {
    return null
  }
}

function isSuccessful(value: unknown): value is Record<string, unknown> {
  return isRecord(value) && value.ok === true
}

function generic(callId: string, name: string, success: boolean): LiveResult {
  const labels: Record<string, [string, string]> = {
    remember: ['记忆操作已完成', '记忆操作未完成'],
    create_todo: ['待办创建已完成', '待办创建未完成'],
    list_todos: ['待办查询已完成', '待办查询未完成'],
    web_search: ['搜索已完成', '搜索未完成'],
    weather_lookup: ['天气查询已完成', '天气查询未完成'],
  }
  const [completed, failed] = labels[name] ?? ['操作已完成', '操作未完成']
  return {
    kind: 'generic',
    callId,
    label: truncate(success ? completed : failed, MAX_LABEL_LENGTH),
    status: success ? 'success' : 'error',
  }
}

function parseMemory(callId: string, result: Record<string, unknown>): LiveResult | null {
  const memory = result.memory
  if (!isRecord(memory)) return null
  const memoryId = identifier(memory.id)
  const title = displayText(memory.user_note)
  if (!memoryId || !title) return null
  return { kind: 'memory_receipt', callId, memoryId, title, status: 'success' }
}

function parseTodo(callId: string, result: Record<string, unknown>): LiveResult | null {
  const todo = result.todo
  if (!isRecord(todo)) return null
  const todoId = identifier(todo.id)
  const title = displayText(todo.title)
  if (!todoId || !title) return null

  const dueValue = todo.due_at
  const dueAt = dueValue === undefined || dueValue === null ? null : finiteNumber(dueValue)
  if (dueAt === null && dueValue !== undefined && dueValue !== null) return null
  return { kind: 'todo_receipt', callId, todoId, title, dueAt, status: 'success' }
}

function validatedRows<T>(
  values: unknown[],
  maximum: number,
  parse: (value: unknown) => T | null,
): T[] | null {
  const count = Math.min(values.length, maximum)
  const rows: T[] = []
  for (let index = 0; index < count; index += 1) {
    if (!Object.hasOwn(values, index)) return null
    const row = parse(values[index])
    if (row === null) return null
    rows.push(row)
  }
  return rows
}

function parseTodoList(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (typeof result.completed !== 'boolean' || !Array.isArray(result.todos)) return null
  const titles = validatedRows(result.todos, MAX_TODO_ROWS, (todo) => {
    if (!isRecord(todo)) return null
    return displayText(todo.title)
  })
  if (!titles) return null
  return {
    kind: 'todo_list',
    callId,
    titles,
    completed: result.completed,
    status: 'success',
  }
}

function parseSearch(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (!isRecord(result.data) || !Array.isArray(result.data.results) || result.data.results.length === 0) {
    return null
  }
  const items = validatedRows(result.data.results, MAX_SEARCH_ROWS, (source) => {
    if (!isRecord(source)) return null
    const title = displayText(source.title)
    const url = sourceUrl(source.url)
    const snippet = displayText(source.snippet, MAX_SNIPPET_LENGTH)
    return title && url && snippet ? { title, url, snippet } : null
  })
  if (!items) return null
  return { kind: 'search', callId, items, status: 'success' }
}

function numericTemperature(value: unknown): number | null {
  const direct = finiteNumber(value)
  if (direct !== null) return direct
  if (typeof value !== 'string' || !/^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value)) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

function parseWeather(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (!isRecord(result.data)) return null
  const directLocation = displayText(result.data.location)
  const directSummary = displayText(result.data.summary)
  const directTemperature = result.data.temperature
  const hasDirectTemperature = directTemperature === undefined || directTemperature === null || finiteNumber(directTemperature) !== null
  if (directLocation && directSummary && hasDirectTemperature) {
    return {
      kind: 'weather',
      callId,
      location: directLocation,
      summary: directSummary,
      temperature: directTemperature === undefined || directTemperature === null ? null : finiteNumber(directTemperature),
      status: 'success',
    }
  }

  if (!isRecord(result.data.location) || !isRecord(result.data.now)) return null
  const location = displayText(result.data.location.name)
  const summary = displayText(result.data.now.text)
  const rawTemperature = result.data.now.temp
  const temperature = rawTemperature === undefined || rawTemperature === null ? null : numericTemperature(rawTemperature)
  if (!location || !summary || (temperature === null && rawTemperature !== undefined && rawTemperature !== null)) {
    return null
  }
  return { kind: 'weather', callId, location, summary, temperature, status: 'success' }
}

export function parseLiveResult(event: ToolCompletion): LiveResult {
  const validCallId = identifier(event.callId)
  const callId = validCallId ?? INVALID_CALL_ID
  const name = typeof event.name === 'string' ? event.name : ''
  if (!validCallId) return generic(callId, name, false)
  if (!isSuccessful(event.result)) return generic(callId, name, false)

  let parsed: LiveResult | null
  switch (name) {
    case 'remember':
      parsed = parseMemory(callId, event.result)
      break
    case 'create_todo':
      parsed = parseTodo(callId, event.result)
      break
    case 'list_todos':
      parsed = parseTodoList(callId, event.result)
      break
    case 'web_search':
      parsed = parseSearch(callId, event.result)
      break
    case 'weather_lookup':
      parsed = parseWeather(callId, event.result)
      break
    default:
      return generic(callId, name, true)
  }
  return parsed ?? generic(callId, name, false)
}
