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
