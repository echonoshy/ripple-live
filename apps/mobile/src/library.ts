export type LibraryApiScope = 'active' | 'archived' | 'all'
export type LibraryView = 'all' | 'pinned' | 'archived' | 'images'
export type LibraryAction =
  | 'pin'
  | 'unpin'
  | 'archive'
  | 'unarchive'
  | 'delete'

export type LibraryItem = {
  id: string
  title: string
  searchableText: string
  timestamp: number
  isPinned: boolean
  archivedAt: number | null
  hasCover?: boolean
}

export type LibraryGroup<T extends LibraryItem = LibraryItem> = {
  label: string
  items: T[]
}

export type LibraryListOptions = {
  scope: LibraryApiScope
  pinned?: boolean
  query: string
  limit: number
}

function timestampMilliseconds(timestamp: number) {
  return timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp
}

function localMidnight(value: Date) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime()
}

function calendarDayOffset(timestamp: number, now: Date) {
  const itemDate = new Date(timestampMilliseconds(timestamp))
  const itemMidnight = localMidnight(itemDate)
  const nowMidnight = localMidnight(now)
  const cursor = new Date(nowMidnight)
  let offset = 0

  while (cursor.getTime() > itemMidnight && offset <= 7) {
    cursor.setDate(cursor.getDate() - 1)
    offset += 1
  }

  return cursor.getTime() === itemMidnight ? offset : 7
}

function dateLabel(timestamp: number, now: Date) {
  const offset = calendarDayOffset(timestamp, now)
  if (offset === 0) return '今天'
  if (offset === 1) return '昨天'
  if (offset <= 6) {
    const date = new Date(timestampMilliseconds(timestamp))
    return `${date.getMonth() + 1}月${date.getDate()}日`
  }
  return '更早'
}

export function matchesLibraryQuery(item: LibraryItem, query: string) {
  const tokens = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)
  if (tokens.length === 0) return true
  const haystack = item.searchableText.toLocaleLowerCase()
  return tokens.every((token) => haystack.includes(token))
}

export function groupLibraryItems<T extends LibraryItem>(
  items: T[],
  now = new Date(),
  view: LibraryView = 'all',
): LibraryGroup<T>[] {
  const visible = items
    .filter((item) => {
      if (view === 'archived') return item.archivedAt !== null
      if (item.archivedAt !== null) return false
      if (view === 'images') return item.hasCover === true
      return view !== 'pinned' || item.isPinned
    })
    .sort((left, right) => right.timestamp - left.timestamp)

  if (view === 'pinned') {
    return visible.length > 0 ? [{ label: '已置顶', items: visible }] : []
  }

  const groups: LibraryGroup<T>[] = []
  const chronological =
    view === 'all' || view === 'images'
      ? visible.filter((item) => !item.isPinned)
      : visible
  if (view === 'all' || view === 'images') {
    const pinned = visible.filter((item) => item.isPinned)
    if (pinned.length > 0) groups.push({ label: '已置顶', items: pinned })
  }

  for (const item of chronological) {
    const label = dateLabel(item.timestamp, now)
    const previous = groups.at(-1)
    if (previous?.label === label) previous.items.push(item)
    else groups.push({ label, items: [item] })
  }
  return groups
}

export function libraryOptionsForView(
  view: LibraryView,
  query: string,
  limit: number,
): LibraryListOptions {
  if (view === 'pinned') {
    return { scope: 'active', pinned: true, query, limit }
  }
  return { scope: view === 'archived' ? 'archived' : 'active', query, limit }
}

export function applyLibraryAction<T extends LibraryItem>(
  items: T[],
  ids: string[],
  action: LibraryAction,
  archivedAt = Date.now() / 1000,
): T[] {
  const selected = new Set(ids)
  if (action === 'delete') return items.filter((item) => !selected.has(item.id))

  return items.map((item) => {
    if (!selected.has(item.id)) return item
    if (action === 'pin') return { ...item, isPinned: true }
    if (action === 'unpin') return { ...item, isPinned: false }
    if (action === 'archive') return { ...item, archivedAt }
    return { ...item, archivedAt: null }
  })
}
