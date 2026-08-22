import assert from 'node:assert/strict'
import test from 'node:test'

import type { UserProfile } from '../src/api.ts'
import {
  isPersonalizationDirty,
  personalizationDraft,
  personalizationUpdatedAt,
} from '../src/components/personalizationModel.ts'

const saved: UserProfile = {
  ai_identity: '长期伙伴',
  user_identity: '独立开发者',
  preferred_name: 'Lake',
  basic_memory: '偏好先看结论',
  updated_at: 1_700_000_000,
}

test('keeps the server-confirmed personalization visible after saving', () => {
  const draft = personalizationDraft(saved)

  assert.deepEqual(draft, {
    ai_identity: '长期伙伴',
    user_identity: '独立开发者',
    preferred_name: 'Lake',
    basic_memory: '偏好先看结论',
  })
  assert.equal(isPersonalizationDirty(draft, saved), false)
})

test('detects local changes and supports restoring the saved profile', () => {
  const changed = { ...personalizationDraft(saved), preferred_name: 'Ripple 用户' }

  assert.equal(isPersonalizationDirty(changed, saved), true)
  assert.equal(isPersonalizationDirty(personalizationDraft(saved), saved), false)
  assert.equal(isPersonalizationDirty(changed, null), true)
})

test('formats second-based server timestamps for the saved summary', () => {
  assert.notEqual(personalizationUpdatedAt(saved.updated_at), '尚未保存')
  assert.equal(personalizationUpdatedAt(null), '尚未保存')
})
