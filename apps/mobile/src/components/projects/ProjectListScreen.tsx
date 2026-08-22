import { ArrowLeft, ChevronRight, FolderKanban, LoaderCircle, Plus } from 'lucide-react'
import type { ProjectRecord } from '../../api'

function projectTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp * 1000))
}

export function ProjectListScreen({
  items,
  scope,
  busy,
  error,
  onBack,
  onScopeChange,
  onCreate,
  onOpen,
  onRetry,
}: {
  items: ProjectRecord[]
  scope: 'active' | 'archived'
  busy: boolean
  error: string
  onBack(): void
  onScopeChange(scope: 'active' | 'archived'): void
  onCreate(): void
  onOpen(project: ProjectRecord): void
  onRetry(): void
}) {
  return (
    <section className="project-screen project-list-screen">
      <header className="project-screen-header">
        <button type="button" aria-label="返回首页" onClick={onBack}><ArrowLeft /></button>
        <div>
          <h1>项目</h1>
          <small>{busy ? '正在读取' : `${items.length} 个项目 · ${scope === 'active' ? '进行中' : '已归档'}`}</small>
        </div>
        <button className="project-create-icon" type="button" aria-label="新建项目" onClick={onCreate}><Plus /></button>
      </header>

      <main className="project-list-content">
        <div className="project-scope-switch" role="tablist" aria-label="项目状态">
          <button
            type="button"
            role="tab"
            aria-selected={scope === 'active'}
            className={scope === 'active' ? 'is-active' : ''}
            onClick={() => onScopeChange('active')}
          >进行中</button>
          <button
            type="button"
            role="tab"
            aria-selected={scope === 'archived'}
            className={scope === 'archived' ? 'is-active' : ''}
            onClick={() => onScopeChange('archived')}
          >已归档</button>
        </div>

        {busy ? (
          <div className="project-loading" aria-label="正在加载项目"><LoaderCircle /><span>读取项目</span></div>
        ) : error ? (
          <div className="project-error" role="alert"><p>{error}</p><button type="button" onClick={onRetry}>重新加载</button></div>
        ) : items.length === 0 ? (
          <div className="project-empty">
            <FolderKanban aria-hidden="true" />
            <small>PROJECT / EMPTY</small>
            <h2>{scope === 'active' ? '还没有项目' : '没有已归档项目'}</h2>
            <p>{scope === 'active' ? '创建一个项目，让对话拥有稳定的背景和规则。' : '归档的项目会显示在这里，也可以随时恢复。'}</p>
            {scope === 'active' ? <button type="button" onClick={onCreate}><Plus />新建项目</button> : null}
          </div>
        ) : (
          <div className="project-list">
            {items.map((project) => (
              <button type="button" key={project.id} onClick={() => onOpen(project)}>
                <span className="project-list-accent" aria-hidden="true" />
                <span className="project-list-copy">
                  <strong>{project.name}</strong>
                  <span>{project.description || '暂未填写项目说明'}</span>
                  <time>最后更新 · {projectTime(project.updated_at)}</time>
                </span>
                <ChevronRight aria-hidden="true" />
              </button>
            ))}
          </div>
        )}
      </main>
    </section>
  )
}
