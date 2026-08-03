import assert from 'node:assert/strict'
import test from 'node:test'

import {
  applyLibraryAction,
  groupLibraryItems,
  libraryOptionsForView,
  matchesLibraryQuery,
} from '../.test-dist/library.js'

const at = (value) => new Date(value).getTime() / 1000

const items = [
  {
    id: 'pinned',
    title: '红茶配料表',
    searchableText: '红茶配料表 水 糖浆 食品添加剂',
    timestamp: at('2026-08-03T10:00:00+08:00'),
    isPinned: true,
    archivedAt: null,
  },
  {
    id: 'today',
    title: '今天',
    searchableText: '今天的聊天',
    timestamp: at('2026-08-03T09:00:00+08:00'),
    isPinned: false,
    archivedAt: null,
  },
  {
    id: 'yesterday',
    title: '昨天',
    searchableText: '昨天的聊天',
    timestamp: at('2026-08-02T18:00:00+08:00'),
    isPinned: false,
    archivedAt: null,
  },
  {
    id: 'recent',
    title: '较早',
    searchableText: '本周较早的聊天',
    timestamp: at('2026-07-29T12:00:00+08:00'),
    isPinned: false,
    archivedAt: null,
  },
  {
    id: 'older',
    title: '更早',
    searchableText: '很久以前的聊天',
    timestamp: at('2026-07-20T12:00:00+08:00'),
    isPinned: false,
    archivedAt: null,
  },
  {
    id: 'archived',
    title: '已归档',
    searchableText: '归档的红茶内容',
    timestamp: at('2026-08-03T08:00:00+08:00'),
    isPinned: true,
    archivedAt: at('2026-08-03T11:00:00+08:00'),
  },
]

test('groups active items by pin and local calendar day without duplicates', () => {
  const groups = groupLibraryItems(
    items,
    new Date('2026-08-03T12:00:00+08:00'),
    'all',
  )

  assert.deepEqual(
    groups.map((group) => group.label),
    ['已置顶', '今天', '昨天', '7月29日', '更早'],
  )
  assert.deepEqual(
    groups.flatMap((group) => group.items).map((item) => item.id),
    ['pinned', 'today', 'yesterday', 'recent', 'older'],
  )
})

test('archived and pinned views only include their own scope', () => {
  assert.deepEqual(
    groupLibraryItems(items, new Date('2026-08-03T12:00:00+08:00'), 'archived')
      .flatMap((group) => group.items)
      .map((item) => item.id),
    ['archived'],
  )
  assert.deepEqual(
    groupLibraryItems(items, new Date('2026-08-03T12:00:00+08:00'), 'pinned')
      .flatMap((group) => group.items)
      .map((item) => item.id),
    ['pinned'],
  )
})

test('matches every normalized query token', () => {
  assert.equal(matchesLibraryQuery(items[0], '红茶 配料'), true)
  assert.equal(matchesLibraryQuery(items[0], '  红茶   糖浆  '), true)
  assert.equal(matchesLibraryQuery(items[0], '不存在'), false)
  assert.equal(matchesLibraryQuery(items[0], '   '), true)
})

test('maps views to server query options', () => {
  assert.deepEqual(libraryOptionsForView('all', '红茶', 50), {
    scope: 'active',
    query: '红茶',
    limit: 50,
  })
  assert.deepEqual(libraryOptionsForView('pinned', '', 50), {
    scope: 'active',
    pinned: true,
    query: '',
    limit: 50,
  })
  assert.deepEqual(libraryOptionsForView('archived', '', 100), {
    scope: 'archived',
    query: '',
    limit: 100,
  })
})

test('applies reversible and delete mutations without changing input', () => {
  const source = items.slice(0, 2)
  const pinned = applyLibraryAction(source, ['today'], 'pin')
  assert.equal(pinned[1].isPinned, true)
  assert.equal(source[1].isPinned, false)

  const archived = applyLibraryAction(source, ['today'], 'archive', 123)
  assert.equal(archived[1].archivedAt, 123)
  assert.equal(applyLibraryAction(archived, ['today'], 'unarchive')[1].archivedAt, null)
  assert.deepEqual(
    applyLibraryAction(source, ['today'], 'delete').map((item) => item.id),
    ['pinned'],
  )
})
