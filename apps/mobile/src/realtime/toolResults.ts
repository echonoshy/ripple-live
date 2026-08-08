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
const CARRIAGE_RETURN = '\r'
const LINE_FEED = '\n'
const graphemeExtension = /^(?:\p{Mark}|\p{Emoji_Modifier}|\p{Variation_Selector})$/u

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function own(record: object, key: PropertyKey) {
  const descriptor = Object.getOwnPropertyDescriptor(record, key)
  return descriptor !== undefined && Object.hasOwn(descriptor, 'value')
}

function ownValue<T>(record: object, key: PropertyKey): T | undefined {
  const descriptor = Object.getOwnPropertyDescriptor(record, key)
  return descriptor !== undefined && Object.hasOwn(descriptor, 'value')
    ? (descriptor.value as T)
    : undefined
}

function hasOwnAccessor(record: object, key: PropertyKey) {
  return Object.hasOwn(record, key) && !own(record, key)
}

function codePoint(value: string) {
  return value.codePointAt(0) ?? 0
}

function isRegionalIndicator(value: string) {
  const point = codePoint(value)
  return point >= 0x1f1e6 && point <= 0x1f1ff
}

type HangulType = 'L' | 'V' | 'T' | 'LV' | 'LVT'

function hangulType(value: string): HangulType | null {
  const point = codePoint(value)
  if ((point >= 0x1100 && point <= 0x115f) || (point >= 0xa960 && point <= 0xa97c)) return 'L'
  if ((point >= 0x1160 && point <= 0x11a7) || (point >= 0xd7b0 && point <= 0xd7c6)) return 'V'
  if ((point >= 0x11a8 && point <= 0x11ff) || (point >= 0xd7cb && point <= 0xd7fb)) return 'T'
  if (point >= 0xac00 && point <= 0xd7a3) return (point - 0xac00) % 28 === 0 ? 'LV' : 'LVT'
  return null
}

function joinsHangul(previous: HangulType | null, next: HangulType | null) {
  if (!previous || !next) return false
  if (previous === 'L') return next === 'L' || next === 'V' || next === 'LV' || next === 'LVT'
  if (previous === 'LV' || previous === 'V') return next === 'V' || next === 'T'
  return (previous === 'LVT' || previous === 'T') && next === 'T'
}

function fallbackGraphemes(value: string) {
  const graphemes: string[] = []
  let current = ''
  let joined = false
  let regionalIndicatorCount = 0
  let currentHangul: HangulType | null = null
  for (const character of value) {
    if (!current) {
      current = character
      regionalIndicatorCount = isRegionalIndicator(character) ? 1 : 0
      currentHangul = hangulType(character)
      continue
    }
    const nextHangul = hangulType(character)
    if (current === CARRIAGE_RETURN && character === LINE_FEED) {
      current += character
      continue
    }
    if (joined || character === ZERO_WIDTH_JOINER || graphemeExtension.test(character)) {
      current += character
      joined = character === ZERO_WIDTH_JOINER
      continue
    }
    if (isRegionalIndicator(character) && regionalIndicatorCount % 2 === 1) {
      current += character
      regionalIndicatorCount += 1
      continue
    }
    if (joinsHangul(currentHangul, nextHangul)) {
      current += character
      currentHangul = nextHangul
      continue
    }
    graphemes.push(current)
    current = character
    joined = false
    regionalIndicatorCount = isRegionalIndicator(character) ? 1 : 0
    currentHangul = nextHangul
  }
  if (current) graphemes.push(current)
  return graphemes
}

function truncate(value: string, maximum: number) {
  const Segmenter = typeof Intl !== 'undefined' ? Intl.Segmenter : undefined
  if (typeof Segmenter === 'function') {
    const graphemes: string[] = []
    const segmenter = new Segmenter(undefined, { granularity: 'grapheme' })
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
    if (
      codePoint !== undefined &&
      (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f))
    ) {
      return true
    }
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
      parsed.password ||
      parsed.href.length > MAX_URL_LENGTH
    ) {
      return null
    }
    return parsed.href
  } catch {
    return null
  }
}

function isSuccessful(value: unknown): value is Record<string, unknown> {
  return isRecord(value) && own(value, 'ok') && ownValue(value, 'ok') === true
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
  if (!own(result, 'memory')) return null
  const memory = ownValue(result, 'memory')
  if (!isRecord(memory) || !own(memory, 'id') || !own(memory, 'user_note')) return null
  const memoryId = identifier(ownValue(memory, 'id'))
  const title = displayText(ownValue(memory, 'user_note'))
  if (!memoryId || !title) return null
  return { kind: 'memory_receipt', callId, memoryId, title, status: 'success' }
}

function parseTodo(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (!own(result, 'todo')) return null
  const todo = ownValue(result, 'todo')
  if (!isRecord(todo) || !own(todo, 'id') || !own(todo, 'title')) return null
  const todoId = identifier(ownValue(todo, 'id'))
  const title = displayText(ownValue(todo, 'title'))
  if (!todoId || !title) return null

  if (hasOwnAccessor(todo, 'due_at')) return null
  const dueValue = own(todo, 'due_at') ? ownValue(todo, 'due_at') : undefined
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
    if (!own(values, index)) return null
    const row = parse(ownValue(values, index))
    if (row === null) return null
    rows.push(row)
  }
  return rows
}

function parseTodoList(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (!own(result, 'completed') || !own(result, 'todos')) return null
  const completed = ownValue(result, 'completed')
  const todos = ownValue(result, 'todos')
  if (typeof completed !== 'boolean' || !Array.isArray(todos)) return null
  const titles = validatedRows(todos, MAX_TODO_ROWS, (todo) => {
    if (!isRecord(todo) || !own(todo, 'title')) return null
    return displayText(ownValue(todo, 'title'))
  })
  if (!titles) return null
  return {
    kind: 'todo_list',
    callId,
    titles,
    completed,
    status: 'success',
  }
}

function parseSearch(callId: string, result: Record<string, unknown>): LiveResult | null {
  if (!own(result, 'data')) return null
  const data = ownValue(result, 'data')
  if (!isRecord(data) || !own(data, 'results')) return null
  const searchResults = ownValue(data, 'results')
  if (!Array.isArray(searchResults) || searchResults.length === 0) {
    return null
  }
  const items = validatedRows(searchResults, MAX_SEARCH_ROWS, (source) => {
    if (!isRecord(source) || !own(source, 'title') || !own(source, 'url') || !own(source, 'snippet')) {
      return null
    }
    const title = displayText(ownValue(source, 'title'))
    const url = sourceUrl(ownValue(source, 'url'))
    const snippet = displayText(ownValue(source, 'snippet'), MAX_SNIPPET_LENGTH)
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
  if (!own(result, 'data')) return null
  const data = ownValue(result, 'data')
  if (!isRecord(data)) return null

  const hasDirectFields = own(data, 'location') && own(data, 'summary')
  const directLocation = hasDirectFields ? displayText(ownValue(data, 'location')) : null
  const directSummary = hasDirectFields ? displayText(ownValue(data, 'summary')) : null
  const hasDirectTemperature = !Object.hasOwn(data, 'temperature') || own(data, 'temperature')
  const directTemperature = own(data, 'temperature') ? ownValue(data, 'temperature') : undefined
  const directTemperatureValid =
    directTemperature === undefined || directTemperature === null || finiteNumber(directTemperature) !== null
  if (directLocation && directSummary && hasDirectTemperature && directTemperatureValid) {
    return {
      kind: 'weather',
      callId,
      location: directLocation,
      summary: directSummary,
      temperature:
        directTemperature === undefined || directTemperature === null
          ? null
          : finiteNumber(directTemperature),
      status: 'success',
    }
  }

  if (!own(data, 'location') || !own(data, 'now')) return null
  const locationData = ownValue(data, 'location')
  const now = ownValue(data, 'now')
  if (!isRecord(locationData) || !isRecord(now) || !own(locationData, 'name') || !own(now, 'text')) {
    return null
  }
  const location = displayText(ownValue(locationData, 'name'))
  const summary = displayText(ownValue(now, 'text'))
  const hasTemperature = !Object.hasOwn(now, 'temp') || own(now, 'temp')
  const rawTemperature = own(now, 'temp') ? ownValue(now, 'temp') : undefined
  const temperature = rawTemperature === undefined || rawTemperature === null ? null : numericTemperature(rawTemperature)
  if (
    !location ||
    !summary ||
    !hasTemperature ||
    (temperature === null && rawTemperature !== undefined && rawTemperature !== null)
  ) {
    return null
  }
  return { kind: 'weather', callId, location, summary, temperature, status: 'success' }
}

export function parseLiveResult(event: ToolCompletion): LiveResult {
  const eventRecord = isRecord(event) ? event : null
  const validCallId = eventRecord && own(eventRecord, 'callId')
    ? identifier(ownValue(eventRecord, 'callId'))
    : null
  const callId = validCallId ?? INVALID_CALL_ID
  const nameValue = eventRecord && own(eventRecord, 'name') ? ownValue(eventRecord, 'name') : undefined
  const name = typeof nameValue === 'string' ? nameValue : ''
  const result = eventRecord && own(eventRecord, 'result') ? ownValue(eventRecord, 'result') : undefined
  if (!validCallId) return generic(callId, name, false)
  if (!isSuccessful(result)) return generic(callId, name, false)

  let parsed: LiveResult | null
  switch (name) {
    case 'remember':
      parsed = parseMemory(callId, result)
      break
    case 'create_todo':
      parsed = parseTodo(callId, result)
      break
    case 'list_todos':
      parsed = parseTodoList(callId, result)
      break
    case 'web_search':
      parsed = parseSearch(callId, result)
      break
    case 'weather_lookup':
      parsed = parseWeather(callId, result)
      break
    default:
      return generic(callId, name, true)
  }
  return parsed ?? generic(callId, name, false)
}
