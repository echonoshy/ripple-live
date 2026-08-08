import assert from 'node:assert/strict'
import test from 'node:test'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { normalizeConversationMessages } from '../src/api'
import {
  activateConversationAction,
  recognizedConversationActions,
} from '../src/conversationActions'
import { ConversationActions } from '../src/components/ConversationActions'

test('normalizes missing legacy actions and non-finite due times', () => {
  const [legacy, current] = normalizeConversationMessages([
    { id: 1, role: 'user', content: '旧消息', created_at: 1, attachments: [] },
    {
      id: 2,
      role: 'user',
      content: '新消息',
      created_at: 2,
      attachments: [],
      actions: [
        { kind: 'todo', target_id: 'todo-1', label: '待办', due_at: Number.NaN },
        { kind: 'memory', target_id: null, label: '坏目标', due_at: null },
      ],
    },
  ])

  assert.deepEqual(legacy.actions, [])
  assert.deepEqual(current.actions, [
    { kind: 'todo', target_id: 'todo-1', label: '待办', due_at: null },
  ])
})

test('normalization rejects inherited actions and contains throwing action proxies', () => {
  const inheritedAction = Object.create({
    kind: 'memory',
    target_id: 'inherited-memory',
    label: '不应出现',
    due_at: null,
  })
  const throwingAction = new Proxy({}, {
    getOwnPropertyDescriptor() { throw new Error('action descriptor trap') },
    get() { throw new Error('action get trap') },
  })

  const [message] = normalizeConversationMessages([{
    id: 3,
    role: 'user',
    content: '包含坏 action',
    created_at: 3,
    attachments: [],
    actions: [
      inheritedAction,
      throwingAction,
      { kind: 'todo', target_id: 'todo-safe', label: '安全待办', due_at: null },
    ],
  }])

  assert.deepEqual(message.actions, [
    { kind: 'todo', target_id: 'todo-safe', label: '安全待办', due_at: null },
  ])
})

test('normalization is total for throwing action arrays and message proxies', () => {
  const throwingActions = new Proxy([], {
    getOwnPropertyDescriptor() { throw new Error('array descriptor trap') },
    get() { throw new Error('array get trap') },
  })
  const throwingMessage = new Proxy({}, {
    getOwnPropertyDescriptor() { throw new Error('message descriptor trap') },
    ownKeys() { throw new Error('message ownKeys trap') },
    get() { throw new Error('message get trap') },
  })

  assert.doesNotThrow(() => normalizeConversationMessages(new Proxy([], {
    getOwnPropertyDescriptor() { throw new Error('messages descriptor trap') },
    get() { throw new Error('messages get trap') },
  })))
  assert.deepEqual(normalizeConversationMessages(new Proxy([], {
    getOwnPropertyDescriptor() { throw new Error('messages descriptor trap') },
    get() { throw new Error('messages get trap') },
  })), [])

  assert.deepEqual(normalizeConversationMessages([
    throwingMessage,
    {
      id: 4,
      role: 'user',
      content: '异常 actions 数组',
      created_at: 4,
      attachments: [],
      actions: throwingActions,
    },
    {
      id: 5,
      role: 'assistant',
      content: '后续有效消息',
      created_at: 5,
      attachments: [],
      actions: [],
    },
  ]), [
    {
      id: 4,
      role: 'user',
      content: '异常 actions 数组',
      created_at: 4,
      attachments: [],
      actions: [],
    },
    {
      id: 5,
      role: 'assistant',
      content: '后续有效消息',
      created_at: 5,
      attachments: [],
      actions: [],
    },
  ])
})

test('recognizes only memory and todo actions with non-empty own targets', () => {
  const inheritedTarget = Object.create({ target_id: 'inherited' })
  inheritedTarget.kind = 'memory'
  inheritedTarget.label = '继承目标'
  inheritedTarget.due_at = null
  const throwingAction = new Proxy({}, {
    getOwnPropertyDescriptor() { throw new Error('descriptor trap') },
    get() { throw new Error('get trap') },
  })

  assert.deepEqual(
    recognizedConversationActions([
      { kind: 'memory', target_id: ' memory-1 ', label: ' 书桌抽屉 ', due_at: null },
      { kind: 'todo', target_id: 'todo-1', label: '', due_at: Number.POSITIVE_INFINITY },
      { kind: 'calendar', target_id: 'calendar-1', label: '日程', due_at: null },
      { kind: 'memory', target_id: '   ', label: '空目标', due_at: null },
      inheritedTarget,
      throwingAction,
      null,
    ]),
    [
      { kind: 'memory', target_id: 'memory-1', label: '书桌抽屉', due_at: null },
      { kind: 'todo', target_id: 'todo-1', label: '待办', due_at: null },
    ],
  )
})

test('renders safe action buttons without treating labels as HTML', () => {
  const markup = renderToStaticMarkup(
    <ConversationActions
      actions={[
        { kind: 'memory', target_id: 'memory-1', label: '<img src=x onerror=alert(1)>', due_at: null },
        { kind: 'todo', target_id: 'todo-1', label: '带充电器', due_at: 1_900_000_000 },
        { kind: 'unknown', target_id: 'unknown-1', label: '不显示', due_at: null },
      ]}
      onActivate={() => {}}
    />,
  )

  assert.match(markup, /aria-label="打开记忆：&lt;img src=x onerror=alert\(1\)&gt;"/)
  assert.match(markup, /&lt;img src=x onerror=alert\(1\)&gt;/)
  assert.match(markup, /带充电器/)
  assert.doesNotMatch(markup, /不显示/)
  assert.doesNotMatch(markup, /<img src=x/)
})

test('activates memory and todo destinations and ignores malformed actions', async () => {
  const opened: string[] = []
  const navigation = {
    openMemory: async (targetId: string) => { opened.push(`memory:${targetId}`) },
    openTodo: async (targetId: string) => { opened.push(`todo:${targetId}`) },
  }

  assert.equal(await activateConversationAction(
    { kind: 'memory', target_id: ' memory-2 ', label: '记忆', due_at: null },
    navigation,
  ), true)
  assert.equal(await activateConversationAction(
    { kind: 'todo', target_id: 'todo-2', label: '待办', due_at: null },
    navigation,
  ), true)
  assert.equal(await activateConversationAction(
    { kind: 'calendar', target_id: 'calendar-2', label: '日程', due_at: null },
    navigation,
  ), false)
  assert.equal(await activateConversationAction(
    { kind: 'memory', target_id: '', label: '坏数据', due_at: null },
    navigation,
  ), false)
  assert.deepEqual(opened, ['memory:memory-2', 'todo:todo-2'])
})
