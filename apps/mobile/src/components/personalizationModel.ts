import type { UserProfile, UserProfileUpdate } from '../api'

export const EMPTY_PERSONALIZATION: UserProfileUpdate = {
  ai_identity: '',
  user_identity: '',
  preferred_name: '',
  basic_memory: '',
}

export const PERSONALIZATION_FIELDS = [
  { key: 'ai_identity', label: 'Ripple 的身份' },
  { key: 'user_identity', label: '你的身份' },
  { key: 'preferred_name', label: '对你的称呼' },
  { key: 'basic_memory', label: '基础资料' },
] as const satisfies ReadonlyArray<{
  key: keyof UserProfileUpdate
  label: string
}>

export function personalizationDraft(profile: UserProfile): UserProfileUpdate {
  return {
    ai_identity: profile.ai_identity,
    user_identity: profile.user_identity,
    preferred_name: profile.preferred_name,
    basic_memory: profile.basic_memory,
  }
}

export function isPersonalizationDirty(
  draft: UserProfileUpdate,
  saved: UserProfile | null,
) {
  const baseline = saved ?? EMPTY_PERSONALIZATION
  return PERSONALIZATION_FIELDS.some(({ key }) => draft[key] !== baseline[key])
}

export function personalizationUpdatedAt(timestamp: number | null) {
  if (timestamp === null) return '尚未保存'
  const milliseconds = timestamp < 1_000_000_000_000 ? timestamp * 1000 : timestamp
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(new Date(milliseconds))
}
