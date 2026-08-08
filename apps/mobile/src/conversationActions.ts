import type { ConversationAction } from './api'

export type RecognizedConversationAction = ConversationAction & {
  kind: 'memory' | 'todo'
}

function ownValue(value: unknown, key: string) {
  if (!value || typeof value !== 'object') return null
  try {
    const descriptor = Object.getOwnPropertyDescriptor(value, key)
    return descriptor && 'value' in descriptor ? descriptor.value : null
  } catch {
    return null
  }
}

function ownString(value: unknown, key: string) {
  const candidate = ownValue(value, key)
  return typeof candidate === 'string' ? candidate : null
}

export function recognizedConversationActions(
  actions: readonly unknown[] | null | undefined,
): RecognizedConversationAction[] {
  if (!Array.isArray(actions)) return []
  return actions.flatMap((action): RecognizedConversationAction[] => {
    const kind = ownString(action, 'kind')
    const targetId = ownString(action, 'target_id')?.trim()
    if ((kind !== 'memory' && kind !== 'todo') || !targetId) return []
    const label = ownString(action, 'label')?.trim() || (kind === 'memory' ? '记忆' : '待办')
    const rawDueAt = ownValue(action, 'due_at')
    return [{
      kind,
      target_id: targetId,
      label,
      due_at: typeof rawDueAt === 'number' && Number.isFinite(rawDueAt)
        ? rawDueAt
        : null,
    }]
  })
}

export type ConversationActionNavigation = {
  openMemory: (targetId: string) => Promise<void> | void
  openTodo: (targetId: string) => Promise<void> | void
}

export async function activateConversationAction(
  action: unknown,
  navigation: ConversationActionNavigation,
) {
  const [recognized] = recognizedConversationActions([action])
  if (!recognized) return false
  if (recognized.kind === 'memory') {
    await navigation.openMemory(recognized.target_id)
  } else {
    await navigation.openTodo(recognized.target_id)
  }
  return true
}
