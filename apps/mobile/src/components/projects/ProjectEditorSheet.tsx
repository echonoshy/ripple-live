import { ChevronDown, X } from 'lucide-react'
import { useState } from 'react'
import type { ProjectCreate, ProjectRecord } from '../../api'
import {
  projectDraft,
  projectDraftError,
  projectPayload,
  type ProjectDraft,
} from './projectDraft'

export function ProjectEditorSheet({
  project,
  busy,
  error,
  onCancel,
  onSave,
}: {
  project?: ProjectRecord
  busy: boolean
  error: string
  onCancel(): void
  onSave(input: ProjectCreate): void
}) {
  const [draft, setDraft] = useState<ProjectDraft>(() => projectDraft(project))
  const validationError = projectDraftError(draft)
  const title = project ? '编辑项目' : '新建项目'

  const update = (field: keyof ProjectDraft, value: string) => {
    setDraft((current) => ({ ...current, [field]: value }))
  }

  return (
    <div
      className="project-editor-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (!busy && event.target === event.currentTarget) onCancel()
      }}
    >
      <section
        className="project-editor-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="project-editor-title"
      >
        <header>
          <div>
            <small>PROJECT / {project ? 'EDIT' : 'NEW'}</small>
            <h2 id="project-editor-title">{title}</h2>
          </div>
          <button type="button" aria-label={`关闭${title}`} disabled={busy} onClick={onCancel}>
            <X aria-hidden="true" />
          </button>
        </header>

        <div className="project-editor-fields">
          <label>
            <span><strong>项目名称</strong><small>{[...draft.name].length} / 80</small></span>
            <input
              autoFocus
              value={draft.name}
              maxLength={80}
              placeholder="例如：Ripple Android 发布"
              disabled={busy}
              onChange={(event) => update('name', event.target.value)}
            />
          </label>

          <label>
            <span><strong>项目说明</strong><small>{[...draft.description].length} / 2000</small></span>
            <textarea
              value={draft.description}
              maxLength={2_000}
              rows={4}
              placeholder="这个项目要解决什么问题？"
              disabled={busy}
              onChange={(event) => update('description', event.target.value)}
            />
          </label>

          <details className="project-rules-editor" open>
            <summary>
              <span><strong>项目规则</strong><small>Ripple 在项目内会一直遵守</small></span>
              <ChevronDown aria-hidden="true" />
            </summary>
            <label>
              <span className="project-rules-count">{[...draft.instructions].length} / 4000</span>
              <textarea
                value={draft.instructions}
                maxLength={4_000}
                rows={7}
                placeholder="例如：优先 Android，不修改 iOS；接口只使用 Responses API。"
                disabled={busy}
                onChange={(event) => update('instructions', event.target.value)}
              />
            </label>
          </details>

          {error ? <p className="project-editor-error" role="alert">{error}</p> : null}
        </div>

        <footer>
          <button type="button" disabled={busy} onClick={onCancel}>取消</button>
          <button
            className="project-editor-primary"
            type="button"
            disabled={busy || Boolean(validationError)}
            onClick={() => onSave(projectPayload(draft))}
          >
            {busy ? '正在保存' : project ? '保存修改' : '创建项目'}
          </button>
        </footer>
      </section>
    </div>
  )
}
