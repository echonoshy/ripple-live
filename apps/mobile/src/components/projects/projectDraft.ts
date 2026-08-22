import type { ConversationSummary, ProjectCreate, ProjectRecord } from '../../api'

export type ProjectDraft = {
  name: string
  description: string
  instructions: string
}

export function projectDraft(project?: ProjectRecord): ProjectDraft {
  return {
    name: project?.name ?? '',
    description: project?.description ?? '',
    instructions: project?.instructions ?? '',
  }
}

export function projectPayload(draft: ProjectDraft): ProjectCreate {
  return {
    name: draft.name.trim(),
    description: draft.description.trim(),
    instructions: draft.instructions.trim(),
  }
}

export function projectDraftError(draft: ProjectDraft) {
  const payload = projectPayload(draft)
  if (!payload.name) return '请输入项目名称'
  if ([...payload.name].length > 80) return '项目名称不能超过 80 个字符'
  if ([...payload.description!].length > 2_000) return '项目说明不能超过 2000 个字符'
  if ([...payload.instructions!].length > 4_000) return '项目规则不能超过 4000 个字符'
  return ''
}

export function hasProjectConversationContent(conversation: ConversationSummary) {
  return conversation.preview.trim().length > 0 || conversation.title.trim() !== '新对话'
}
