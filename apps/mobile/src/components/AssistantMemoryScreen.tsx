import {
  ArrowLeft,
  Brain,
  BriefcaseBusiness,
  FileText,
  Heart,
  List,
  LoaderCircle,
  Plus,
  RefreshCw,
  Repeat2,
  Search,
  Sparkles,
  UserRound,
  UsersRound,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import {
  createMemoryFact,
  deleteMemoryFact,
  memoryFacts,
  updateMemoryFact,
  type MemoryFact,
  type MemoryFactKind,
} from '../api'
import './AssistantMemoryScreen.css'

const CATEGORIES: Array<{
  kind: MemoryFactKind
  label: string
  description: string
  icon: typeof Brain
}> = [
  { kind: 'identity', label: '关于我', description: '身份与长期背景', icon: UserRound },
  { kind: 'preference', label: '偏好', description: '喜欢与不喜欢', icon: Heart },
  { kind: 'relationship', label: '人物关系', description: '重要的人与关系', icon: UsersRound },
  { kind: 'habit', label: '习惯', description: '稳定的习惯与做法', icon: Repeat2 },
  { kind: 'context', label: '长期事项', description: '持续关注的工作与生活背景', icon: BriefcaseBusiness },
  { kind: 'other', label: '其他', description: '其他希望 Ripple 记住的事', icon: Sparkles },
]

type EditorState = {
  fact?: MemoryFact
  kind: MemoryFactKind
  summary: string
}

type MemoryView = 'all' | 'preference' | 'relationship' | 'context'

const MEMORY_FILTERS: Array<{ value: MemoryView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'preference', label: '偏好' },
  { value: 'relationship', label: '人物' },
  { value: 'context', label: '计划' },
]

function formatMemoryTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
  }).format(new Date(timestamp * 1000))
}

export function AssistantMemoryScreen({
  server,
  token,
  onBack,
}: {
  server: string
  token: string
  onBack(): void
}) {
  const [items, setItems] = useState<MemoryFact[]>([])
  const [query, setQuery] = useState('')
  const [view, setView] = useState<MemoryView>('all')
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState('')
  const [editor, setEditor] = useState<EditorState | null>(null)
  const [editorBusy, setEditorBusy] = useState(false)
  const [editorError, setEditorError] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState(false)
  const [reload, setReload] = useState(0)

  useEffect(() => {
    let active = true
    setBusy(true)
    setError('')
    void memoryFacts(server, token)
      .then((facts) => {
        if (active) setItems(facts)
      })
      .catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : '无法读取记忆')
      })
      .finally(() => {
        if (active) setBusy(false)
      })
    return () => {
      active = false
    }
  }, [reload, server, token])

  const visibleItems = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase('zh-CN')
    return items.filter((item) => {
      if (view !== 'all' && item.kind !== view) return false
      return !normalized || item.summary.toLocaleLowerCase('zh-CN').includes(normalized)
    })
  }, [items, query, view])

  const groupedItems = useMemo(
    () => CATEGORIES.map((category) => ({
      ...category,
      items: visibleItems.filter((item) => item.kind === category.kind),
    })).filter((group) => group.items.length > 0),
    [visibleItems],
  )

  const openCreate = () => {
    setEditor({ kind: 'other', summary: '' })
    setEditorError('')
    setDeleteConfirm(false)
  }

  const openEdit = (fact: MemoryFact) => {
    setEditor({ fact, kind: fact.kind, summary: fact.summary })
    setEditorError('')
    setDeleteConfirm(false)
  }

  const save = async () => {
    if (!editor || editorBusy) return
    const summary = editor.summary.trim()
    if (!summary) {
      setEditorError('请写下希望 Ripple 记住的内容')
      return
    }
    setEditorBusy(true)
    setEditorError('')
    try {
      const saved = editor.fact
        ? await updateMemoryFact(server, token, editor.fact.id, {
            kind: editor.kind,
            summary,
          })
        : await createMemoryFact(server, token, editor.kind, summary)
      setItems((current) => [saved, ...current.filter((item) => item.id !== saved.id)])
      setEditor(null)
    } catch (reason) {
      setEditorError(reason instanceof Error ? reason.message : '无法保存记忆')
    } finally {
      setEditorBusy(false)
    }
  }

  const remove = async () => {
    if (!editor?.fact || editorBusy) return
    setEditorBusy(true)
    setEditorError('')
    try {
      await deleteMemoryFact(server, token, editor.fact.id)
      setItems((current) => current.filter((item) => item.id !== editor.fact?.id))
      setEditor(null)
    } catch (reason) {
      setEditorError(reason instanceof Error ? reason.message : '无法删除记忆')
    } finally {
      setEditorBusy(false)
    }
  }

  return (
    <section className="assistant-memory-screen">
      <header className="assistant-memory-header">
        <button type="button" aria-label="返回首页" onClick={onBack}><ArrowLeft /></button>
        <div className="assistant-memory-path" aria-label={`长期记忆，共 ${items.length} 项`}>
          <span>MEMORY</span><i>/</i><strong>{String(items.length).padStart(2, '0')}</strong>
        </div>
        <button type="button" aria-label="添加记忆" onClick={openCreate}><Plus /></button>
      </header>

      <main className="assistant-memory-content">
        <div className="assistant-memory-title">
          <h1>记忆</h1>
          <span>MEMORY / {String(items.length).padStart(2, '0')}</span>
        </div>

        <div className="assistant-memory-toolbar">
          <label className="assistant-memory-search">
            <Search aria-hidden="true" />
            <input
              aria-label="搜索记忆"
              value={query}
              placeholder="搜索记忆"
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <nav className="assistant-memory-filters" aria-label="记忆分类">
            {MEMORY_FILTERS.map((filter) => (
              <button key={filter.value} className={view === filter.value ? 'is-active' : ''} type="button" onClick={() => setView(filter.value)}>{filter.label}</button>
            ))}
          </nav>
        </div>

        <div className={`assistant-memory-status${error ? ' is-error' : ''}`}>
          <RefreshCw aria-hidden="true" />
          <span>{busy ? 'SYNCING' : error ? 'SYNC ERROR' : 'SYNCED'} · {visibleItems.length} ITEMS</span>
          <List aria-hidden="true" />
        </div>

        {busy ? (
          <div className="assistant-memory-loading"><LoaderCircle /><span>正在读取记忆</span></div>
        ) : null}
        {error ? (
          <div className="assistant-memory-error" role="alert">
            <span>{error}</span>
            <button type="button" onClick={() => setReload((value) => value + 1)}><RefreshCw />重试</button>
          </div>
        ) : null}
        {!busy && !error && visibleItems.length === 0 ? (
          <section className="assistant-memory-empty">
            <FileText />
            <h2>{query ? '没有找到相关记忆' : '还没有长期记忆'}</h2>
            <p>{query ? '换一个更短的关键词试试。' : 'Ripple 会在相处中记住稳定的信息。'}</p>
            {!query ? <button type="button" onClick={openCreate}><Plus />添加记忆</button> : null}
          </section>
        ) : null}

        <div className="assistant-memory-groups">
          {groupedItems.map((group) => {
            const Icon = group.icon
            return (
              <section key={group.kind} className="assistant-memory-group">
                <header><Icon /><div><h2>{group.label}</h2><p>{group.description}</p></div><span>{group.items.length}</span></header>
                <div className="assistant-memory-list">
                  {group.items.map((fact) => (
                    <button type="button" key={fact.id} onClick={() => openEdit(fact)}>
                      <span>{fact.summary}</span>
                      <small>
                        {fact.source === 'conversation' ? '来自对话' : '手动添加'}
                        {fact.scope_type === 'project' ? ' · 仅当前项目' : ''}
                        {' · '}{formatMemoryTime(fact.updated_at)}
                      </small>
                    </button>
                  ))}
                </div>
              </section>
            )
          })}
        </div>
      </main>

      {editor ? (
        <div className="assistant-memory-editor-layer">
          <button className="assistant-memory-editor-backdrop" type="button" aria-label="关闭编辑" onClick={() => setEditor(null)} />
          <section className="assistant-memory-editor" role="dialog" aria-modal="true" aria-labelledby="assistant-memory-editor-title">
            <header>
              <div><small>MEMORY ENTRY</small><h2 id="assistant-memory-editor-title">{editor.fact ? '编辑记忆' : '添加记忆'}</h2></div>
              <button type="button" aria-label="关闭" onClick={() => setEditor(null)}><X /></button>
            </header>
            <label>
              <span>类别</span>
              <select value={editor.kind} onChange={(event) => setEditor({ ...editor, kind: event.target.value as MemoryFactKind })}>
                {CATEGORIES.map((category) => <option key={category.kind} value={category.kind}>{category.label}</option>)}
              </select>
            </label>
            <label>
              <span>希望 Ripple 记住什么</span>
              <textarea
                autoFocus
                rows={5}
                maxLength={500}
                value={editor.summary}
                placeholder="例如：我不吃香菜，希望回答先给结论。"
                onChange={(event) => setEditor({ ...editor, summary: event.target.value })}
              />
              <small>{editor.summary.length} / 500</small>
            </label>
            {editorError ? <p className="assistant-memory-editor-error" role="alert">{editorError}</p> : null}
            <footer>
              {editor.fact ? (
                deleteConfirm ? (
                  <button className="is-delete" type="button" disabled={editorBusy} onClick={() => void remove()}>确认删除</button>
                ) : (
                  <button className="is-quiet" type="button" disabled={editorBusy} onClick={() => setDeleteConfirm(true)}>删除</button>
                )
              ) : <span />}
              <button className="is-primary" type="button" disabled={editorBusy} onClick={() => void save()}>{editorBusy ? '保存中…' : '保存记忆'}</button>
            </footer>
          </section>
        </div>
      ) : null}
    </section>
  )
}
