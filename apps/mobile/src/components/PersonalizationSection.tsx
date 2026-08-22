import { useEffect, useState } from 'react'
import { ArrowLeft, ChevronRight } from 'lucide-react'
import {
  updateUserProfile,
  userProfile,
  type UserProfile,
  type UserProfileUpdate,
} from '../api'
import {
  EMPTY_PERSONALIZATION,
  isPersonalizationDirty,
  PERSONALIZATION_FIELDS,
  personalizationDraft,
  personalizationUpdatedAt,
} from './personalizationModel'
import './PersonalizationSection.css'

export function PersonalizationSection({
  server,
  token,
  onBack,
}: {
  server: string
  token: string
  onBack?(): void
}) {
  const [draft, setDraft] = useState<UserProfileUpdate>(EMPTY_PERSONALIZATION)
  const [savedProfile, setSavedProfile] = useState<UserProfile | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')
  const [currentExpanded, setCurrentExpanded] = useState(false)

  useEffect(() => {
    let active = true
    setLoading(true)
    setError('')
    void userProfile(server, token)
      .then((profile) => {
        if (!active) return
        setSavedProfile(profile)
        setDraft(personalizationDraft(profile))
      })
      .catch((reason: unknown) => {
        if (active) {
          setError(reason instanceof Error ? reason.message : '无法加载个性化设定')
        }
      })
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [server, token])

  const updateField = (field: keyof UserProfileUpdate, value: string) => {
    setDraft((current) => ({ ...current, [field]: value }))
    setMessage('')
    setError('')
  }

  const save = async () => {
    setSaving(true)
    setMessage('')
    setError('')
    try {
      const profile = await updateUserProfile(server, token, draft)
      setSavedProfile(profile)
      setDraft(personalizationDraft(profile))
      setMessage('已保存，将从下一轮对话开始生效')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '保存失败，请重试')
    } finally {
      setSaving(false)
    }
  }

  const dirty = isPersonalizationDirty(draft, savedProfile)

  const restoreSaved = () => {
    if (!savedProfile) return
    setDraft(personalizationDraft(savedProfile))
    setMessage('已恢复为当前保存的配置')
    setError('')
  }

  const profileSummary = savedProfile
    ? `Ripple：${savedProfile.ai_identity || '未设置'} · 称呼：${savedProfile.preferred_name || '未设置'}`
    : loading ? '正在读取配置' : '尚未设置'

  const statusLabel = loading ? 'SYNCING' : dirty ? 'EDITING' : 'SYNCED'

  return (
    <section className="personalization-workspace" aria-labelledby="personalization-heading">
      <header className="personalization-header">
        <button type="button" aria-label="返回首页" onClick={onBack}><ArrowLeft /></button>
        <div className="personalization-path" aria-label={`配置状态：${statusLabel}`}>
          <span>PROFILE</span><i>/</i><strong>{statusLabel}</strong>
        </div>
        <span />
      </header>

      <main className="personalization-content">
        <h1 id="personalization-heading">个性化</h1>

        <section className="personalization-current-v2" aria-labelledby="personalization-current-heading">
          <h2 id="personalization-current-heading">当前配置</h2>
          <button type="button" aria-expanded={currentExpanded} onClick={() => setCurrentExpanded((value) => !value)}>
            <span>{profileSummary}</span>
            <ChevronRight aria-hidden="true" />
          </button>
          {currentExpanded && savedProfile ? (
            <div className="personalization-current-detail">
              <dl>
                {PERSONALIZATION_FIELDS.map(({ key, label }) => (
                  <div key={key}><dt>{label}</dt><dd>{savedProfile[key] || '未设置'}</dd></div>
                ))}
              </dl>
              <p>最近保存：{personalizationUpdatedAt(savedProfile.updated_at)}</p>
            </div>
          ) : null}
        </section>

        <form className="personalization-form-v2" aria-busy={loading || saving} onSubmit={(event) => { event.preventDefault(); void save() }}>
          <label>
            <span>Ripple 的身份</span>
            <textarea
              value={draft.ai_identity}
              maxLength={2000}
              rows={2}
              disabled={loading}
              placeholder="温柔、直接、会主动提醒我的长期伙伴"
              onChange={(event) => updateField('ai_identity', event.target.value)}
            />
          </label>
          <label>
            <span>你的身份</span>
            <textarea
              value={draft.user_identity}
              maxLength={2000}
              rows={2}
              disabled={loading}
              placeholder="独立开发者，正在做 AI 陪伴产品"
              onChange={(event) => updateField('user_identity', event.target.value)}
            />
          </label>
          <label>
            <span>希望怎么称呼你</span>
            <input
              value={draft.preferred_name}
              maxLength={80}
              disabled={loading}
              placeholder="名字、昵称或称呼"
              onChange={(event) => updateField('preferred_name', event.target.value)}
            />
          </label>
          <details className="personalization-basic">
            <summary>
              <span><strong>基础资料</strong><small>常驻地点、偏好与沟通习惯</small></span>
              <ChevronRight aria-hidden="true" />
            </summary>
            <label>
              <span className="sr-only">基础资料</span>
              <textarea
                value={draft.basic_memory}
                maxLength={4000}
                rows={3}
                disabled={loading}
                placeholder="例如：我常驻上海；偏好先看结论；不吃香菜"
                onChange={(event) => updateField('basic_memory', event.target.value)}
              />
            </label>
          </details>

          {error ? <p className="personalization-feedback-v2 is-error" role="alert">{error}</p> : null}
          {message ? <p className="personalization-feedback-v2" role="status">{message}</p> : null}

          <div className="personalization-actions-v2">
            <button className="personalization-restore-v2" type="button" disabled={loading || saving || !dirty || !savedProfile} onClick={restoreSaved}>恢复</button>
            <button className="personalization-save-v2" type="submit" disabled={loading || saving || !dirty}>
              {saving ? '正在保存…' : '保存配置'}
            </button>
          </div>
        </form>
      </main>
    </section>
  )
}
