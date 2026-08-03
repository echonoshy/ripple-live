import { isTauri } from '@tauri-apps/api/core'
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'
import type { TodoItem } from './api'

const notifiedKey = 'ripple-reminded-todos'

function notifiedTodoIds() {
  try {
    return new Set<string>(JSON.parse(localStorage.getItem(notifiedKey) ?? '[]'))
  } catch {
    return new Set<string>()
  }
}

export async function notifyDueTodos(items: TodoItem[]) {
  if (!isTauri()) return
  const due = items.filter(
    (item) => item.due_at !== null && item.due_at <= Date.now() / 1000,
  )
  if (due.length === 0) return

  let permitted = await isPermissionGranted()
  if (!permitted) permitted = (await requestPermission()) === 'granted'
  if (!permitted) return

  const seen = notifiedTodoIds()
  for (const item of due) {
    if (seen.has(item.id)) continue
    sendNotification({ title: 'Ripple Live 提醒', body: item.title })
    seen.add(item.id)
  }
  localStorage.setItem(notifiedKey, JSON.stringify([...seen]))
}
