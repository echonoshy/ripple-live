import { Brain, ListTodo as ListChecks } from 'lucide-react'
import React from 'react'
import type { ConversationAction } from '../api'
import { recognizedConversationActions } from '../conversationActions'

export function ConversationActions({
  actions,
  onActivate,
}: {
  actions: readonly ConversationAction[] | null | undefined
  onActivate: (action: ConversationAction) => Promise<unknown> | void
}) {
  const recognized = recognizedConversationActions(actions)
  if (recognized.length === 0) return null

  return (
    <React.Fragment>
      <div className="conversation-actions" aria-label="对话中保存的内容">
        {recognized.map((action) => (
          <button
            key={`${action.kind}:${action.target_id}`}
            type="button"
            aria-label={`打开${action.kind === 'memory' ? '记忆' : '待办'}：${action.label}`}
            onClick={() => void onActivate(action)}
          >
            {action.kind === 'memory' ? <Brain aria-hidden="true" /> : <ListChecks aria-hidden="true" />}
            <span>{action.label}</span>
          </button>
        ))}
      </div>
    </React.Fragment>
  )
}
