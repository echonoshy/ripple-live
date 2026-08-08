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

type OwnDataRead =
  | { kind: 'data'; value: unknown }
  | { kind: 'missing' }
  | { kind: 'invalid' }

function isObject(value: unknown): value is object {
  return typeof value === 'object' && value !== null
}

function isArray(value: unknown): value is unknown[] {
  try {
    return Array.isArray(value)
  } catch {
    return false
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return isObject(value) && !isArray(value)
}

function ownValue(record: unknown, key: PropertyKey): OwnDataRead {
  if (!isObject(record)) return { kind: 'invalid' }
  try {
    const descriptor = Object.getOwnPropertyDescriptor(record, key)
    if (descriptor === undefined) return { kind: 'missing' }
    return Object.hasOwn(descriptor, 'value')
      ? { kind: 'data', value: descriptor.value }
      : { kind: 'invalid' }
  } catch {
    return { kind: 'invalid' }
  }
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
      regionalIndicatorCount = 0
      currentHangul = null
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
  if (!isRecord(value)) return false
  const ok = ownValue(value, 'ok')
  return ok.kind === 'data' && ok.value === true
}

function generic(callId: string, name: string, success: boolean): LiveResult {
  let completed = '操作已完成'
  let failed = '操作未完成'
  switch (name) {
    case 'remember':
      completed = '记忆操作已完成'
      failed = '记忆操作未完成'
      break
    case 'create_todo':
      completed = '待办创建已完成'
      failed = '待办创建未完成'
      break
    case 'list_todos':
      completed = '待办查询已完成'
      failed = '待办查询未完成'
      break
    case 'web_search':
      completed = '搜索已完成'
      failed = '搜索未完成'
      break
    case 'weather_lookup':
      completed = '天气查询已完成'
      failed = '天气查询未完成'
      break
  }
  return {
    kind: 'generic',
    callId,
    label: truncate(success ? completed : failed, MAX_LABEL_LENGTH),
    status: success ? 'success' : 'error',
  }
}

function parseMemory(callId: string, result: Record<string, unknown>): LiveResult | null {
  const memoryField = ownValue(result, 'memory')
  if (memoryField.kind !== 'data' || !isRecord(memoryField.value)) return null
  const id = ownValue(memoryField.value, 'id')
  const userNote = ownValue(memoryField.value, 'user_note')
  if (id.kind !== 'data' || userNote.kind !== 'data') return null
  const memoryId = identifier(id.value)
  const title = displayText(userNote.value)
  if (!memoryId || !title) return null
  return { kind: 'memory_receipt', callId, memoryId, title, status: 'success' }
}

function parseTodo(callId: string, result: Record<string, unknown>): LiveResult | null {
  const todoField = ownValue(result, 'todo')
  if (todoField.kind !== 'data' || !isRecord(todoField.value)) return null
  const id = ownValue(todoField.value, 'id')
  const todoTitle = ownValue(todoField.value, 'title')
  if (id.kind !== 'data' || todoTitle.kind !== 'data') return null
  const todoId = identifier(id.value)
  const title = displayText(todoTitle.value)
  if (!todoId || !title) return null

  const due = ownValue(todoField.value, 'due_at')
  if (due.kind === 'invalid') return null
  const dueValue = due.kind === 'data' ? due.value : undefined
  const dueAt = dueValue === undefined || dueValue === null ? null : finiteNumber(dueValue)
  if (dueAt === null && dueValue !== undefined && dueValue !== null) return null
  return { kind: 'todo_receipt', callId, todoId, title, dueAt, status: 'success' }
}

function validatedRows<T>(
  values: unknown[],
  maximum: number,
  parse: (value: unknown) => T | null,
): T[] | null {
  const length = ownValue(values, 'length')
  if (
    length.kind !== 'data' ||
    typeof length.value !== 'number' ||
    !Number.isSafeInteger(length.value) ||
    length.value < 0
  ) {
    return null
  }
  const count = Math.min(length.value, maximum)
  const rows: T[] = []
  for (let index = 0; index < count; index += 1) {
    const value = ownValue(values, index)
    if (value.kind !== 'data') return null
    const row = parse(value.value)
    if (row === null) return null
    rows.push(row)
  }
  return rows
}

function parseTodoList(callId: string, result: Record<string, unknown>): LiveResult | null {
  const completed = ownValue(result, 'completed')
  const todos = ownValue(result, 'todos')
  if (completed.kind !== 'data' || todos.kind !== 'data' || typeof completed.value !== 'boolean' || !isArray(todos.value)) {
    return null
  }
  const titles = validatedRows(todos.value, MAX_TODO_ROWS, (todo) => {
    if (!isRecord(todo)) return null
    const title = ownValue(todo, 'title')
    return title.kind === 'data' ? displayText(title.value) : null
  })
  if (!titles) return null
  return {
    kind: 'todo_list',
    callId,
    titles,
    completed: completed.value,
    status: 'success',
  }
}

function parseSearch(callId: string, result: Record<string, unknown>): LiveResult | null {
  const data = ownValue(result, 'data')
  if (data.kind !== 'data' || !isRecord(data.value)) return null
  const searchResults = ownValue(data.value, 'results')
  if (searchResults.kind !== 'data' || !isArray(searchResults.value)) return null
  const length = ownValue(searchResults.value, 'length')
  if (
    length.kind !== 'data' ||
    typeof length.value !== 'number' ||
    !Number.isSafeInteger(length.value) ||
    length.value < 0
  ) {
    return null
  }

  const items: Array<{ title: string; url: string; snippet: string }> = []
  const urls = new Set<string>()
  for (
    let index = 0;
    index < length.value && items.length < MAX_SEARCH_ROWS;
    index += 1
  ) {
    const sourceField = ownValue(searchResults.value, index)
    if (sourceField.kind !== 'data') return null
    const source = sourceField.value
    if (!isRecord(source)) return null
    const titleField = ownValue(source, 'title')
    const urlField = ownValue(source, 'url')
    const snippetField = ownValue(source, 'snippet')
    if (titleField.kind !== 'data' || urlField.kind !== 'data' || snippetField.kind !== 'data') return null
    const title = displayText(titleField.value)
    const url = sourceUrl(urlField.value)
    const snippet = displayText(snippetField.value, MAX_SNIPPET_LENGTH)
    if (!title || !url || !snippet) return null
    if (urls.has(url)) continue
    urls.add(url)
    items.push({ title, url, snippet })
  }
  if (items.length === 0) return null
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
  const data = ownValue(result, 'data')
  if (data.kind !== 'data' || !isRecord(data.value)) return null

  const directLocationField = ownValue(data.value, 'location')
  const directSummaryField = ownValue(data.value, 'summary')
  const directTemperatureField = ownValue(data.value, 'temperature')
  const directLocation =
    directLocationField.kind === 'data' ? displayText(directLocationField.value) : null
  const directSummary =
    directSummaryField.kind === 'data' ? displayText(directSummaryField.value) : null
  const hasDirectTemperature = directTemperatureField.kind !== 'invalid'
  const directTemperature =
    directTemperatureField.kind === 'data' ? directTemperatureField.value : undefined
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

  if (directLocationField.kind !== 'data') return null
  const nowField = ownValue(data.value, 'now')
  if (nowField.kind !== 'data' || !isRecord(directLocationField.value) || !isRecord(nowField.value)) {
    return null
  }
  const locationName = ownValue(directLocationField.value, 'name')
  const weatherText = ownValue(nowField.value, 'text')
  const temperatureField = ownValue(nowField.value, 'temp')
  if (locationName.kind !== 'data' || weatherText.kind !== 'data' || temperatureField.kind === 'invalid') {
    return null
  }
  const location = displayText(locationName.value)
  const summary = displayText(weatherText.value)
  const rawTemperature = temperatureField.kind === 'data' ? temperatureField.value : undefined
  const temperature = rawTemperature === undefined || rawTemperature === null ? null : numericTemperature(rawTemperature)
  if (
    !location ||
    !summary ||
    (temperature === null && rawTemperature !== undefined && rawTemperature !== null)
  ) {
    return null
  }
  return { kind: 'weather', callId, location, summary, temperature, status: 'success' }
}

function parseResult(event: ToolCompletion): LiveResult {
  const eventRecord = isRecord(event) ? event : null
  const callIdField = eventRecord ? ownValue(eventRecord, 'callId') : { kind: 'invalid' as const }
  const validCallId = callIdField.kind === 'data' ? identifier(callIdField.value) : null
  const callId = validCallId ?? INVALID_CALL_ID
  const nameField = eventRecord ? ownValue(eventRecord, 'name') : { kind: 'invalid' as const }
  const name = nameField.kind === 'data' && typeof nameField.value === 'string' ? nameField.value : ''
  const resultField = eventRecord ? ownValue(eventRecord, 'result') : { kind: 'invalid' as const }
  const result = resultField.kind === 'data' ? resultField.value : undefined
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

export function parseLiveResult(event: ToolCompletion): LiveResult {
  try {
    return parseResult(event)
  } catch {
    return { kind: 'generic', callId: INVALID_CALL_ID, label: '操作未完成', status: 'error' }
  }
}
