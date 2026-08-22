import { useEffect, useState } from 'react'
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

export function PersonalizationSection({
  server,
  token,
}: {
  server: string
  token: string
}) {
  const [draft, setDraft] = useState<UserProfileUpdate>(EMPTY_PERSONALIZATION)
  const [savedProfile, setSavedProfile] = useState<UserProfile | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')

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

  return (
    <section
      className="profile-section personalization-section"
      aria-labelledby="personalization-heading"
    >
      <h2 id="personalization-heading">个性化</h2>
      <div className="personalization-intro">
        <strong>让 Ripple 更懂你</strong>
        <p>这些设定会在每次对话中生效，不会混入通话里保存的视觉记忆。</p>
      </div>
      <section className="personalization-current" aria-labelledby="personalization-current-heading">
        <div className="personalization-current-header">
          <div>
            <span className="personalization-kicker">CURRENT.CONFIG</span>
            <h3 id="personalization-current-heading">当前配置</h3>
          </div>
          <span className={`personalization-save-state${dirty ? ' is-dirty' : ''}`}>
            {loading ? '同步中' : dirty ? '有未保存修改' : '已同步'}
          </span>
        </div>
        {savedProfile ? (
          <>
            <dl className="personalization-current-list">
              {PERSONALIZATION_FIELDS.map(({ key, label }) => (
                <div key={key}>
                  <dt>{label}</dt>
                  <dd className={savedProfile[key] ? '' : 'is-empty'}>
                    {savedProfile[key] || '未设置'}
                  </dd>
                </div>
              ))}
            </dl>
            <p className="personalization-updated-at">
              最近保存：{personalizationUpdatedAt(savedProfile.updated_at)}
            </p>
          </>
        ) : (
          <p className="personalization-current-empty">
            {loading ? '正在从服务端读取配置…' : '还没有可显示的配置'}
          </p>
        )}
      </section>
      <div className="personalization-form" aria-busy={loading || saving}>
        <div className="personalization-form-heading">
          <span className="personalization-kicker">EDIT.CONFIG</span>
          <strong>编辑配置</strong>
        </div>
        <label>
          <span>Ripple 的身份</span>
          <small>例如：一位温柔、直接、会主动提醒我的长期伙伴</small>
          <textarea
            value={draft.ai_identity}
            maxLength={2000}
            rows={3}
            disabled={loading}
            placeholder="描述 Ripple 应该是谁、如何与你相处"
            onChange={(event) => updateField('ai_identity', event.target.value)}
          />
        </label>
        <label>
          <span>你的身份</span>
          <small>帮助 Ripple 理解你的角色、背景或当前阶段</small>
          <textarea
            value={draft.user_identity}
            maxLength={2000}
            rows={3}
            disabled={loading}
            placeholder="例如：我是独立开发者，正在做一款 AI 陪伴产品"
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
        <label>
          <span>基础资料</span>
          <small>填写长期稳定的信息、偏好与沟通习惯；临时事件仍放在记忆中</small>
          <textarea
            value={draft.basic_memory}
            maxLength={4000}
            rows={4}
            disabled={loading}
            placeholder="例如：我常驻上海；偏好先看结论；不吃香菜"
            onChange={(event) => updateField('basic_memory', event.target.value)}
          />
        </label>
        {error ? (
          <p className="personalization-feedback is-error" role="alert">{error}</p>
        ) : null}
        {message ? (
          <p className="personalization-feedback" role="status">{message}</p>
        ) : null}
        <div className="personalization-actions">
          <button
            className="personalization-restore"
            type="button"
            disabled={loading || saving || !dirty || !savedProfile}
            onClick={restoreSaved}
          >
            恢复为已保存
          </button>
          <button
            className="personalization-save"
            type="button"
            disabled={loading || saving || !dirty}
            onClick={() => void save()}
          >
            {saving ? '正在保存…' : loading ? '正在加载…' : dirty ? '保存设定' : '配置已同步'}
          </button>
        </div>
      </div>
    </section>
  )
}
