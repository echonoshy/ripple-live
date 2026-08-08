import assert from 'node:assert/strict'
import test from 'node:test'

import { parseLiveResult } from '../src/realtime/toolResults.ts'

test('creates a memory receipt only for a successful validated memory', () => {
  const result = parseLiveResult({
    callId: 'call-1',
    name: 'remember',
    result: { ok: true, memory: { id: 'mem_1', user_note: '65W 充电器' } },
  })

  assert.deepEqual(result, {
    kind: 'memory_receipt',
    callId: 'call-1',
    memoryId: 'mem_1',
    title: '65W 充电器',
    status: 'success',
  })
})

test('creates a todo receipt with an optional due time', () => {
  const result = parseLiveResult({
    callId: 'call-2',
    name: 'create_todo',
    result: { ok: true, todo: { id: 'todo_1', title: '带充电器', due_at: 1786323600 } },
  })

  assert.deepEqual(result, {
    kind: 'todo_receipt',
    callId: 'call-2',
    todoId: 'todo_1',
    title: '带充电器',
    dueAt: 1786323600,
    status: 'success',
  })
})

test('uses null for an omitted todo due time', () => {
  const result = parseLiveResult({
    callId: 'call-3',
    name: 'create_todo',
    result: { ok: true, todo: { id: 'todo_2', title: '准备会议' } },
  })

  assert.deepEqual(result, {
    kind: 'todo_receipt',
    callId: 'call-3',
    todoId: 'todo_2',
    title: '准备会议',
    dueAt: null,
    status: 'success',
  })
})

test('creates a bounded validated todo list', () => {
  const result = parseLiveResult({
    callId: 'call-4',
    name: 'list_todos',
    result: {
      ok: true,
      completed: false,
      todos: [
        { title: '一' },
        { title: '二' },
        { title: '三' },
        { title: '四' },
        { title: '五' },
        { title: '六' },
      ],
    },
  })

  assert.deepEqual(result, {
    kind: 'todo_list',
    callId: 'call-4',
    titles: ['一', '二', '三', '四', '五'],
    completed: false,
    status: 'success',
  })
})

test('bounds web search cards to three validated sources', () => {
  const result = parseLiveResult({
    callId: 'call-5',
    name: 'web_search',
    result: {
      ok: true,
      data: {
        results: [
          { title: 'One', url: 'https://one.example', snippet: 'First' },
          { title: 'Two', url: 'https://two.example', snippet: 'Second' },
          { title: 'Three', url: 'https://three.example', snippet: 'Third' },
          { title: 'Four', url: 'https://four.example', snippet: 'Fourth' },
        ],
      },
    },
  })

  assert.deepEqual(result, {
    kind: 'search',
    callId: 'call-5',
    items: [
      { title: 'One', url: 'https://one.example/', snippet: 'First' },
      { title: 'Two', url: 'https://two.example/', snippet: 'Second' },
      { title: 'Three', url: 'https://three.example/', snippet: 'Third' },
    ],
    status: 'success',
  })
})

test('creates a weather card from a validated external payload', () => {
  const result = parseLiveResult({
    callId: 'call-6',
    name: 'weather_lookup',
    result: {
      ok: true,
      data: { location: '上海', summary: '晴', temperature: 31 },
    },
  })

  assert.deepEqual(result, {
    kind: 'weather',
    callId: 'call-6',
    location: '上海',
    summary: '晴',
    temperature: 31,
    status: 'success',
  })
})

test('creates a weather card from the gateway weather payload shape', () => {
  const result = parseLiveResult({
    callId: 'call-6b',
    name: 'weather_lookup',
    result: {
      ok: true,
      data: {
        location: { name: '上海' },
        now: { text: '多云', temp: '29.5' },
      },
    },
  })

  assert.deepEqual(result, {
    kind: 'weather',
    callId: 'call-6b',
    location: '上海',
    summary: '多云',
    temperature: 29.5,
    status: 'success',
  })
})

test('reports only a deterministic bounded generic result for unknown tools', () => {
  const result = parseLiveResult({
    callId: 'call-7',
    name: 'custom_tool',
    result: { ok: true, html: '<article>untrusted</article>' },
  })

  assert.deepEqual(result, {
    kind: 'generic',
    callId: 'call-7',
    label: '操作已完成',
    status: 'success',
  })
})

test('never trusts malformed, failed, or incomplete known payloads', () => {
  const invalidEvents = [
    { callId: 'bad-1', name: 'remember', result: '<script>' },
    { callId: 'bad-2', name: 'remember', result: { ok: true, memory: { id: 'mem', user_note: '  ' } } },
    { callId: 'bad-3', name: 'remember', result: { ok: true, memory: { id: ' ', user_note: '有效标题' } } },
    { callId: 'bad-4', name: 'create_todo', result: { ok: false } },
    { callId: 'bad-5', name: 'create_todo', result: { ok: true, todo: { id: ' ', title: 'x' } } },
    { callId: 'bad-6', name: 'create_todo', result: { ok: true, todo: { id: 'todo', title: 'x', due_at: Infinity } } },
    { callId: 'bad-7', name: 'list_todos', result: { ok: true, completed: 'false', todos: [] } },
    { callId: 'bad-8', name: 'list_todos', result: { ok: true, completed: false, todos: [{ title: ' ' }] } },
    { callId: 'bad-9', name: 'web_search', result: { ok: true, data: { results: [{ title: 'x', url: 'javascript:alert(1)', snippet: 'x' }] } } },
    { callId: 'bad-10', name: 'weather_lookup', result: { ok: true, data: { location: '上海', summary: '晴', temperature: Number.NaN } } },
  ]

  for (const event of invalidEvents) {
    const result = parseLiveResult(event)
    assert.equal(result.kind, 'generic')
    assert.equal(result.status, 'error')
    assert.equal(result.label.includes('<'), false)
  }
})

test('does not create a success receipt for a whitespace-only call id', () => {
  const result = parseLiveResult({
    callId: '   ',
    name: 'remember',
    result: { ok: true, memory: { id: 'mem_3', user_note: '充电器' } },
  })

  assert.deepEqual(result, {
    kind: 'generic',
    callId: 'unknown-call',
    label: '记忆操作未完成',
    status: 'error',
  })
})

test('rejects empty display rows and credentialed or non-absolute search URLs', () => {
  const invalidResults = [
    {
      ok: true,
      data: { results: [{ title: ' ', url: 'https://example.com', snippet: 'x' }] },
    },
    {
      ok: true,
      data: { results: [{ title: 'x', url: 'https://user:pass@example.com', snippet: 'x' }] },
    },
    {
      ok: true,
      data: { results: [{ title: 'x', url: '/relative', snippet: 'x' }] },
    },
    {
      ok: true,
      data: { results: [{ title: 'x', url: 'ftp://example.com', snippet: 'x' }] },
    },
  ]

  for (const result of invalidResults) {
    assert.deepEqual(parseLiveResult({ callId: 'bad-url', name: 'web_search', result }), {
      kind: 'generic',
      callId: 'bad-url',
      label: '搜索未完成',
      status: 'error',
    })
  }
})

test('truncates bounded display strings', () => {
  const longTitle = 'x'.repeat(121)
  const result = parseLiveResult({
    callId: 'call-8',
    name: 'remember',
    result: { ok: true, memory: { id: 'mem_2', user_note: longTitle } },
  })

  assert.equal(result.kind, 'memory_receipt')
  if (result.kind === 'memory_receipt') assert.equal(result.title.length, 120)

  const searchResult = parseLiveResult({
    callId: 'call-9',
    name: 'web_search',
    result: {
      ok: true,
      data: {
        results: [{ title: longTitle, url: 'https://example.com', snippet: 'x'.repeat(241) }],
      },
    },
  })
  assert.equal(searchResult.kind, 'search')
  if (searchResult.kind === 'search') {
    assert.equal(searchResult.items[0]?.title.length, 120)
    assert.equal(searchResult.items[0]?.snippet.length, 240)
  }
})

test('rejects sparse displayed todo and search rows', () => {
  const sparseTodos = new Array(1)
  const sparseSearchResults = new Array(1)

  assert.deepEqual(
    parseLiveResult({
      callId: 'sparse-todos',
      name: 'list_todos',
      result: { ok: true, completed: false, todos: sparseTodos },
    }),
    {
      kind: 'generic',
      callId: 'sparse-todos',
      label: '待办查询未完成',
      status: 'error',
    },
  )
  assert.deepEqual(
    parseLiveResult({
      callId: 'sparse-search',
      name: 'web_search',
      result: { ok: true, data: { results: sparseSearchResults } },
    }),
    {
      kind: 'generic',
      callId: 'sparse-search',
      label: '搜索未完成',
      status: 'error',
    },
  )
})

test('rejects inherited displayed todo and search rows', () => {
  const inheritedTodos = new Array<unknown>(1)
  const todoPrototype = Object.create(Array.prototype) as Record<number, unknown>
  todoPrototype[0] = { title: '继承待办' }
  Object.setPrototypeOf(inheritedTodos, todoPrototype)

  const inheritedSearchResults = new Array<unknown>(1)
  const searchPrototype = Object.create(Array.prototype) as Record<number, unknown>
  searchPrototype[0] = { title: '继承结果', url: 'https://example.com', snippet: '不应展示' }
  Object.setPrototypeOf(inheritedSearchResults, searchPrototype)

  assert.equal(
    parseLiveResult({
      callId: 'inherited-todos',
      name: 'list_todos',
      result: { ok: true, completed: false, todos: inheritedTodos },
    }).kind,
    'generic',
  )
  assert.equal(
    parseLiveResult({
      callId: 'inherited-search',
      name: 'web_search',
      result: { ok: true, data: { results: inheritedSearchResults } },
    }).kind,
    'generic',
  )
})

test('ignores sparse rows beyond the display caps', () => {
  const todos = [
    { title: '一' },
    { title: '二' },
    { title: '三' },
    { title: '四' },
    { title: '五' },
  ]
  todos.length = 6
  const searchResults = [
    { title: 'One', url: 'https://one.example', snippet: 'First' },
    { title: 'Two', url: 'https://two.example', snippet: 'Second' },
    { title: 'Three', url: 'https://three.example', snippet: 'Third' },
  ]
  searchResults.length = 4

  const todoResult = parseLiveResult({
    callId: 'capped-todos',
    name: 'list_todos',
    result: { ok: true, completed: false, todos },
  })
  assert.equal(todoResult.kind, 'todo_list')
  if (todoResult.kind === 'todo_list') assert.equal(todoResult.titles.length, 5)

  const searchResult = parseLiveResult({
    callId: 'capped-search',
    name: 'web_search',
    result: { ok: true, data: { results: searchResults } },
  })
  assert.equal(searchResult.kind, 'search')
  if (searchResult.kind === 'search') assert.equal(searchResult.items.length, 3)
})

test('preserves unbounded opaque identifiers exactly', () => {
  const longCallId = `call-${'c'.repeat(200)}`
  const longMemoryId = `memory-${'m'.repeat(200)}`
  const longTodoId = `todo-${'t'.repeat(200)}`

  assert.deepEqual(
    parseLiveResult({ callId: longCallId, name: 'unknown', result: { ok: true } }),
    { kind: 'generic', callId: longCallId, label: '操作已完成', status: 'success' },
  )
  assert.equal(
    parseLiveResult({
      callId: 'long-memory',
      name: 'remember',
      result: { ok: true, memory: { id: longMemoryId, user_note: '保留 ID' } },
    }).memoryId,
    longMemoryId,
  )
  assert.equal(
    parseLiveResult({
      callId: 'long-todo',
      name: 'create_todo',
      result: { ok: true, todo: { id: longTodoId, title: '保留 ID' } },
    }).todoId,
    longTodoId,
  )
})

test('keeps display truncation on complete grapheme clusters', () => {
  const combining = 'e\u0301'
  const combiningTitle = `${combining.repeat(120)}z`
  const zwjTitle = `${'a'.repeat(119)}👩‍💻z`

  const combiningResult = parseLiveResult({
    callId: 'grapheme-combining',
    name: 'remember',
    result: { ok: true, memory: { id: 'mem-grapheme-1', user_note: combiningTitle } },
  })
  assert.equal(combiningResult.kind, 'memory_receipt')
  if (combiningResult.kind === 'memory_receipt') {
    assert.equal(combiningResult.title, combining.repeat(120))
  }

  const zwjResult = parseLiveResult({
    callId: 'grapheme-zwj',
    name: 'remember',
    result: { ok: true, memory: { id: 'mem-grapheme-2', user_note: zwjTitle } },
  })
  assert.equal(zwjResult.kind, 'memory_receipt')
  if (zwjResult.kind === 'memory_receipt') {
    assert.equal(zwjResult.title, `${'a'.repeat(119)}👩‍💻`)
  }
})

test('rejects every C1 control character in opaque identifiers', () => {
  for (let codePoint = 0x80; codePoint <= 0x9f; codePoint += 1) {
    const c1 = String.fromCodePoint(codePoint)
    assert.deepEqual(
      parseLiveResult({
        callId: `call${c1}`,
        name: 'remember',
        result: { ok: true, memory: { id: 'mem', user_note: '标题' } },
      }),
      { kind: 'generic', callId: 'unknown-call', label: '记忆操作未完成', status: 'error' },
    )
    assert.equal(
      parseLiveResult({
        callId: 'c1-memory',
        name: 'remember',
        result: { ok: true, memory: { id: `mem${c1}`, user_note: '标题' } },
      }).kind,
      'generic',
    )
    assert.equal(
      parseLiveResult({
        callId: 'c1-todo',
        name: 'create_todo',
        result: { ok: true, todo: { id: `todo${c1}`, title: '标题' } },
      }).kind,
      'generic',
    )
  }
})

test('requires own data properties at every validated payload layer', () => {
  const inheritedEnvelope = Object.create({ ok: true })
  inheritedEnvelope.memory = { id: 'mem-envelope', user_note: '标题' }

  const inheritedMemory = Object.create({ id: 'mem-inherited' })
  inheritedMemory.user_note = '标题'

  const inheritedTodo = Object.create({ title: '继承标题' })
  inheritedTodo.id = 'todo-inherited'

  const inheritedTodoList = Object.create({ completed: false })
  inheritedTodoList.ok = true
  inheritedTodoList.todos = [{ title: '待办' }]

  const inheritedData = Object.create({
    results: [{ title: '来源', url: 'https://example.com', snippet: '摘要' }],
  })

  const inheritedSource = Object.create({ title: '继承来源' })
  inheritedSource.url = 'https://example.com'
  inheritedSource.snippet = '摘要'

  const inheritedLocation = Object.create({ name: '上海' })
  const nestedWeather = {
    ok: true,
    data: { location: inheritedLocation, now: { text: '晴', temp: '20' } },
  }

  const events = [
    { callId: 'own-envelope', name: 'remember', result: inheritedEnvelope },
    { callId: 'own-memory', name: 'remember', result: { ok: true, memory: inheritedMemory } },
    { callId: 'own-todo', name: 'create_todo', result: { ok: true, todo: inheritedTodo } },
    { callId: 'own-list', name: 'list_todos', result: inheritedTodoList },
    { callId: 'own-data', name: 'web_search', result: { ok: true, data: inheritedData } },
    {
      callId: 'own-source',
      name: 'web_search',
      result: { ok: true, data: { results: [inheritedSource] } },
    },
    { callId: 'own-weather', name: 'weather_lookup', result: nestedWeather },
  ]

  for (const event of events) {
    const result = parseLiveResult(event)
    assert.equal(result.kind, 'generic')
    assert.equal(result.status, 'error')
  }
})

test('rejects accessors without invoking their getters', () => {
  const memory: Record<string, unknown> = { user_note: '标题' }
  let getterCalls = 0
  Object.defineProperty(memory, 'id', {
    enumerable: true,
    get: () => {
      getterCalls += 1
      throw new Error('untrusted getter executed')
    },
  })

  const result = parseLiveResult({
    callId: 'accessor',
    name: 'remember',
    result: { ok: true, memory },
  })

  assert.deepEqual(result, {
    kind: 'generic',
    callId: 'accessor',
    label: '记忆操作未完成',
    status: 'error',
  })
  assert.equal(getterCalls, 0)
})

test('rejects source URLs whose canonical form exceeds the URL cap', () => {
  const url = `https://example.com/${'界'.repeat(700)}`
  const result = parseLiveResult({
    callId: 'long-canonical-url',
    name: 'web_search',
    result: { ok: true, data: { results: [{ title: '来源', url, snippet: '摘要' }] } },
  })

  assert.deepEqual(result, {
    kind: 'generic',
    callId: 'long-canonical-url',
    label: '搜索未完成',
    status: 'error',
  })
})

async function withForcedFallback<T>(callback: (parse: typeof parseLiveResult) => T) {
  const descriptor = Object.getOwnPropertyDescriptor(Intl, 'Segmenter')
  Object.defineProperty(Intl, 'Segmenter', { configurable: true, value: undefined })
  try {
    const module = await import(`../src/realtime/toolResults.ts?fallback=${Date.now()}`)
    return callback(module.parseLiveResult)
  } finally {
    if (descriptor) Object.defineProperty(Intl, 'Segmenter', descriptor)
    else delete (Intl as Record<string, unknown>).Segmenter
  }
}

test('fallback segmentation keeps flags, Hangul, and CRLF complete at the boundary', async () => {
  const prefix = 'a'.repeat(119)
  const cases = [
    { callId: 'fallback-flag', title: `${prefix}🇨🇳z`, expected: `${prefix}🇨🇳` },
    { callId: 'fallback-hangul', title: `${prefix}\u1100\u1161z`, expected: `${prefix}\u1100\u1161` },
    { callId: 'fallback-crlf', title: `${prefix}\r\nz`, expected: `${prefix}\r\n` },
  ]

  for (const item of cases) {
    const result = await withForcedFallback((parse) =>
      parse({
        callId: item.callId,
        name: 'remember',
        result: { ok: true, memory: { id: `${item.callId}-id`, user_note: item.title } },
      }),
    )
    assert.equal(result.kind, 'memory_receipt')
    if (result.kind === 'memory_receipt') assert.equal(result.title, item.expected)
  }
})

test('uses deterministic generic output for dangerous or non-string unknown tool names', () => {
  const names: unknown[] = [
    '__proto__',
    'constructor',
    'toString',
    'valueOf',
    Symbol('tool'),
    { toString: () => { throw new Error('must not coerce') } },
  ]

  for (const name of names) {
    const result = parseLiveResult({ callId: 'unknown-name', name, result: { ok: true } } as ToolCompletion)
    assert.deepEqual(result, {
      kind: 'generic',
      callId: 'unknown-name',
      label: '操作已完成',
      status: 'success',
    })
  }
})

test('is total for throwing and revoked proxies at envelope and nested levels', () => {
  const throwingEnvelope = new Proxy({}, {
    getOwnPropertyDescriptor: () => { throw new Error('descriptor trap') },
  })
  const revokedEnvelope = Proxy.revocable({}, {})
  revokedEnvelope.revoke()
  const revokedResult = Proxy.revocable({}, {})
  revokedResult.revoke()
  const throwingRows = new Proxy([{ title: '待办' }], {
    getOwnPropertyDescriptor: (target, key) => {
      if (key === 'length') throw new Error('length trap')
      return Reflect.getOwnPropertyDescriptor(target, key)
    },
  })

  const events: unknown[] = [
    throwingEnvelope,
    revokedEnvelope.proxy,
    { callId: 'revoked-result', name: 'remember', result: revokedResult.proxy },
    {
      callId: 'proxy-array',
      name: 'list_todos',
      result: { ok: true, completed: false, todos: throwingRows },
    },
  ]

  for (const event of events) {
    assert.doesNotThrow(() => parseLiveResult(event as ToolCompletion))
    assert.equal(parseLiveResult(event as ToolCompletion).kind, 'generic')
  }
})

test('reads each validated field descriptor once', () => {
  let idDescriptorReads = 0
  const memory = new Proxy(
    { id: 'one-read', user_note: '标题' },
    {
      getOwnPropertyDescriptor: (target, key) => {
        if (key === 'id') idDescriptorReads += 1
        return Reflect.getOwnPropertyDescriptor(target, key)
      },
    },
  )

  const result = parseLiveResult({
    callId: 'one-read',
    name: 'remember',
    result: { ok: true, memory },
  })
  assert.equal(result.kind, 'memory_receipt')
  assert.equal(idDescriptorReads, 1)
})

test('forced fallback resets RI and Hangul adjacency after extension marks', async () => {
  const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
  const prefix = 'a'.repeat(119)
  const cases = [
    `${prefix}🇨\u0301🇳z`,
    `${prefix}\u1100\u0301\u1161z`,
  ]

  for (const title of cases) {
    const expected = Array.from(segmenter.segment(title), (item) => item.segment).slice(0, 120).join('')
    const result = await withForcedFallback((parse) =>
      parse({
        callId: `fallback-mark-${title.length}`,
        name: 'remember',
        result: { ok: true, memory: { id: `fallback-mark-${title.length}`, user_note: title } },
      }),
    )
    assert.equal(result.kind, 'memory_receipt')
    if (result.kind === 'memory_receipt') assert.equal(result.title, expected)
  }
})
