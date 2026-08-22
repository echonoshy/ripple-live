import {
  Archive,
  ArrowLeft,
  BookOpenText,
  ExternalLink,
  FileText,
  FolderOpen,
  Globe2,
  LoaderCircle,
  MoreHorizontal,
  NotebookPen,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import {
  assetBlob,
  createLibraryResource,
  deleteLibraryResource,
  libraryResource,
  libraryResources,
  projects,
  updateLibraryResource,
  type LibraryResource,
  type LibraryResourceType,
  type ProjectRecord,
} from '../api'
import './LibraryResourcesScreen.css'

type ResourceView = 'all' | LibraryResourceType | 'archived'

type EditorState = {
  resource?: LibraryResource
  type: LibraryResourceType
  title: string
  content: string
  url: string
  projectId: string
  file: File | null
}

const FILTERS: Array<{ value: ResourceView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'link', label: '链接' },
  { value: 'note', label: '笔记' },
  { value: 'file', label: '文件' },
]

const RESOURCE_META: Record<LibraryResourceType, {
  label: string
  description: string
  icon: typeof FileText
}> = {
  file: { label: '上传文件', description: 'PDF、TXT 或 Markdown', icon: Upload },
  link: { label: '保存网页', description: '保留一个可信来源链接', icon: Globe2 },
  note: { label: '新建笔记', description: '写下可供对话引用的内容', icon: NotebookPen },
}

function resourceIcon(type: LibraryResourceType) {
  if (type === 'file') return FileText
  if (type === 'link') return Globe2
  return BookOpenText
}

function formatResourceDate(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    year: new Date(timestamp * 1000).getFullYear() === new Date().getFullYear()
      ? undefined
      : 'numeric',
  }).format(new Date(timestamp * 1000))
}

function inferMimeType(file: File) {
  if (file.type) return file.type
  if (file.name.toLocaleLowerCase().endsWith('.md')) return 'text/markdown'
  if (file.name.toLocaleLowerCase().endsWith('.txt')) return 'text/plain'
  if (file.name.toLocaleLowerCase().endsWith('.pdf')) return 'application/pdf'
  return ''
}

function arrayBufferToBase64(buffer: ArrayBuffer) {
  const bytes = new Uint8Array(buffer)
  let binary = ''
  const chunkSize = 32_768
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize))
  }
  return btoa(binary)
}

function editorForType(type: LibraryResourceType): EditorState {
  return { type, title: '', content: '', url: '', projectId: '', file: null }
}

function editorForResource(resource: LibraryResource): EditorState {
  return {
    resource,
    type: resource.resource_type,
    title: resource.title,
    content: resource.content,
    url: resource.source_url ?? '',
    projectId: resource.project_id ?? '',
    file: null,
  }
}

export function LibraryResourcesScreen({
  server,
  token,
  onBack,
}: {
  server: string
  token: string
  onBack(): void
}) {
  const [items, setItems] = useState<LibraryResource[]>([])
  const [projectItems, setProjectItems] = useState<ProjectRecord[]>([])
  const [view, setView] = useState<ResourceView>('all')
  const [query, setQuery] = useState('')
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState('')
  const [reload, setReload] = useState(0)
  const [addMenuOpen, setAddMenuOpen] = useState(false)
  const [editor, setEditor] = useState<EditorState | null>(null)
  const [editorBusy, setEditorBusy] = useState(false)
  const [editorError, setEditorError] = useState('')
  const [selected, setSelected] = useState<LibraryResource | null>(null)
  const [detailBusy, setDetailBusy] = useState(false)
  const [detailError, setDetailError] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    let active = true
    setBusy(true)
    setError('')
    const archived = view === 'archived'
    const type = view === 'all' || archived ? undefined : view
    void libraryResources(server, token, { archived, type })
      .then((resources) => {
        if (active) setItems(resources)
      })
      .catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : '无法读取资料库')
      })
      .finally(() => {
        if (active) setBusy(false)
      })
    return () => {
      active = false
    }
  }, [reload, server, token, view])

  useEffect(() => {
    let active = true
    void projects(server, token, { scope: 'active', query: '', limit: 100 })
      .then((nextProjects) => {
        if (active) setProjectItems(nextProjects)
      })
      .catch(() => {
        if (active) setProjectItems([])
      })
    return () => {
      active = false
    }
  }, [server, token])

  const visibleItems = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase('zh-CN')
    if (!normalized) return items
    return items.filter((item) => [
      item.title,
      item.content,
      item.source_url ?? '',
      item.project_name ?? '',
    ].some((value) => value.toLocaleLowerCase('zh-CN').includes(normalized)))
  }, [items, query])

  const openResource = async (resource: LibraryResource) => {
    setSelected(resource)
    setDetailBusy(true)
    setDetailError('')
    setDeleteConfirm(false)
    try {
      setSelected(await libraryResource(server, token, resource.id))
    } catch (reason) {
      setDetailError(reason instanceof Error ? reason.message : '无法读取资料详情')
    } finally {
      setDetailBusy(false)
    }
  }

  const openCreate = (type: LibraryResourceType) => {
    setAddMenuOpen(false)
    setEditor(editorForType(type))
    setEditorError('')
  }

  const saveEditor = async () => {
    if (!editor || editorBusy) return
    const title = editor.title.trim()
    if (!title) {
      setEditorError('请填写资料标题')
      return
    }
    if (editor.type === 'note' && !editor.content.trim()) {
      setEditorError('请填写笔记内容')
      return
    }
    if (editor.type === 'link' && !editor.url.trim()) {
      setEditorError('请填写网页链接')
      return
    }
    if (editor.type === 'file' && !editor.resource && !editor.file) {
      setEditorError('请选择需要上传的文件')
      return
    }
    setEditorBusy(true)
    setEditorError('')
    try {
      let saved: LibraryResource
      if (editor.resource) {
        saved = await updateLibraryResource(server, token, editor.resource.id, {
          title,
          ...(editor.type === 'note' ? { content: editor.content.trim() } : {}),
          ...(editor.type === 'link' ? {
            content: editor.content.trim(),
            url: editor.url.trim(),
          } : {}),
        })
      } else if (editor.type === 'file' && editor.file) {
        const mimeType = inferMimeType(editor.file)
        if (!['application/pdf', 'text/plain', 'text/markdown'].includes(mimeType)) {
          throw new Error('当前仅支持 PDF、TXT 和 Markdown 文件')
        }
        if (editor.file.size > 10 * 1024 * 1024) {
          throw new Error('文件不能超过 10MB')
        }
        saved = await createLibraryResource(server, token, {
          type: 'file',
          title,
          project_id: editor.projectId || undefined,
          file_name: editor.file.name,
          mime_type: mimeType,
          data_base64: arrayBufferToBase64(await editor.file.arrayBuffer()),
        })
      } else {
        saved = await createLibraryResource(server, token, {
          type: editor.type,
          title,
          content: editor.content.trim() || undefined,
          url: editor.url.trim() || undefined,
          project_id: editor.projectId || undefined,
        })
      }
      setItems((current) => {
        const withoutSaved = current.filter((item) => item.id !== saved.id)
        const belongsInView = view === 'all' || view === saved.resource_type
        return belongsInView ? [saved, ...withoutSaved] : withoutSaved
      })
      setSelected(saved)
      setEditor(null)
    } catch (reason) {
      setEditorError(reason instanceof Error ? reason.message : '无法保存资料')
    } finally {
      setEditorBusy(false)
    }
  }

  const toggleArchive = async () => {
    if (!selected || detailBusy) return
    setDetailBusy(true)
    setDetailError('')
    try {
      const saved = await updateLibraryResource(server, token, selected.id, {
        archived: selected.archived_at === null,
      })
      setItems((current) => current.filter((item) => item.id !== saved.id))
      setSelected(null)
    } catch (reason) {
      setDetailError(reason instanceof Error ? reason.message : '无法更新资料')
    } finally {
      setDetailBusy(false)
    }
  }

  const removeSelected = async () => {
    if (!selected || detailBusy) return
    setDetailBusy(true)
    setDetailError('')
    try {
      await deleteLibraryResource(server, token, selected.id)
      setItems((current) => current.filter((item) => item.id !== selected.id))
      setSelected(null)
    } catch (reason) {
      setDetailError(reason instanceof Error ? reason.message : '无法删除资料')
    } finally {
      setDetailBusy(false)
    }
  }

  const openFile = async () => {
    if (!selected?.asset_id || detailBusy) return
    setDetailBusy(true)
    setDetailError('')
    try {
      const blob = await assetBlob(server, token, selected.asset_id)
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.target = '_blank'
      anchor.rel = 'noopener noreferrer'
      anchor.click()
      window.setTimeout(() => URL.revokeObjectURL(url), 60_000)
    } catch (reason) {
      setDetailError(reason instanceof Error ? reason.message : '无法打开文件')
    } finally {
      setDetailBusy(false)
    }
  }

  return (
    <section className="resource-library-screen">
      <header className="resource-library-header">
        <button type="button" aria-label="返回首页" onClick={onBack}><ArrowLeft /></button>
        <div className="resource-library-path" aria-label={`资料库，共 ${items.length} 项资料`}>
          <span>LIBRARY</span><i>/</i><strong>{String(items.length).padStart(2, '0')}</strong>
        </div>
        <button type="button" aria-label="添加资料" onClick={() => setAddMenuOpen(true)}><Plus /></button>
      </header>

      <main className="resource-library-content">
        <div className="resource-library-title">
          <h1>资料库</h1>
          <span>/ {String(items.length).padStart(2, '0')}</span>
        </div>

        <div className="resource-library-toolbar">
          <label className="resource-library-search">
            <Search aria-hidden="true" />
            <input aria-label="搜索资料" value={query} placeholder="搜索资料" onChange={(event) => setQuery(event.target.value)} />
          </label>

          <nav className="resource-library-filters" aria-label="资料类型">
            {FILTERS.map((filter) => (
              <button key={filter.value} className={view === filter.value ? 'is-active' : ''} type="button" onClick={() => setView(filter.value)}>{filter.label}</button>
            ))}
            <button className={`resource-library-archive-filter${view === 'archived' ? ' is-active' : ''}`} type="button" aria-label="查看已归档资料" title="已归档" onClick={() => setView('archived')}><Archive /></button>
          </nav>
        </div>

        {busy ? <div className="resource-library-state"><LoaderCircle /><span>正在读取资料</span></div> : null}
        {error ? (
          <div className="resource-library-state is-error" role="alert"><span>{error}</span><button type="button" onClick={() => setReload((value) => value + 1)}><RefreshCw />重试</button></div>
        ) : null}
        {!busy && !error && visibleItems.length === 0 ? (
          <section className="resource-library-empty">
            <FolderOpen />
            <h2>{query ? '没有找到相关资料' : view === 'archived' ? '没有已归档资料' : '暂无资料'}</h2>
            <p>{query ? '换一个更短的关键词试试。' : view === 'archived' ? '归档后的资料会显示在这里。' : '添加可信资料，让 Ripple 在对话中引用。'}</p>
            {!query && view !== 'archived' ? <button type="button" onClick={() => setAddMenuOpen(true)}><Plus />添加资料</button> : null}
          </section>
        ) : null}

        {visibleItems.length > 0 ? (
          <div className="resource-library-list">
            {visibleItems.map((resource) => {
              const Icon = resourceIcon(resource.resource_type)
              return (
                <button key={resource.id} type="button" onClick={() => void openResource(resource)}>
                  <span className={`resource-library-type is-${resource.resource_type}`}><Icon /></span>
                  <span className="resource-library-row-copy">
                    <strong>{resource.title}</strong>
                    <small>{resource.content || resource.source_url || (resource.status === 'stored' ? '文件已保存，等待正文解析' : '暂无摘要')}</small>
                    <em>{RESOURCE_META[resource.resource_type].label.replace('上传', '').replace('保存', '').replace('新建', '')}{resource.project_name ? ` · ${resource.project_name}` : ' · 个人资料'} · {formatResourceDate(resource.updated_at)}</em>
                  </span>
                  <MoreHorizontal />
                </button>
              )
            })}
          </div>
        ) : null}
      </main>

      {addMenuOpen ? (
        <div className="resource-library-layer">
          <button className="resource-library-backdrop" type="button" aria-label="关闭添加菜单" onClick={() => setAddMenuOpen(false)} />
          <section className="resource-library-add-menu" role="dialog" aria-modal="true" aria-labelledby="resource-library-add-title">
            <header><div><h2 id="resource-library-add-title">添加资料</h2><p>选择资料来源</p></div><button type="button" aria-label="关闭" onClick={() => setAddMenuOpen(false)}><X /></button></header>
            {(['file', 'link', 'note'] as const).map((type) => {
              const entry = RESOURCE_META[type]
              const Icon = entry.icon
              return <button key={type} type="button" onClick={() => openCreate(type)}><Icon /><span><strong>{entry.label}</strong><small>{entry.description}</small></span></button>
            })}
          </section>
        </div>
      ) : null}

      {selected && !editor ? (
        <div className="resource-library-layer">
          <button className="resource-library-backdrop" type="button" aria-label="关闭资料详情" onClick={() => setSelected(null)} />
          <section className="resource-library-detail" role="dialog" aria-modal="true" aria-labelledby="resource-library-detail-title">
            <header><div><small>{RESOURCE_META[selected.resource_type].label}</small><h2 id="resource-library-detail-title">{selected.title}</h2></div><button type="button" aria-label="关闭" onClick={() => setSelected(null)}><X /></button></header>
            <div className="resource-library-detail-meta"><span>{selected.project_name ?? '个人资料'}</span><span>{formatResourceDate(selected.updated_at)}</span>{selected.status === 'stored' ? <span>待解析</span> : null}</div>
            {detailBusy ? <div className="resource-library-detail-loading"><LoaderCircle />处理中</div> : null}
            {selected.resource_type === 'link' && selected.source_url ? <a href={selected.source_url} target="_blank" rel="noreferrer"><Globe2 />{selected.source_url}<ExternalLink /></a> : null}
            {selected.content ? <div className="resource-library-detail-content">{selected.content}</div> : null}
            {selected.resource_type === 'file' && !selected.content ? <div className="resource-library-file-note"><FileText /><p>文件已经安全保存。PDF 正文解析会在下一阶段接入。</p></div> : null}
            {detailError ? <p className="resource-library-detail-error" role="alert">{detailError}</p> : null}
            <footer>
              <div>
                <button type="button" disabled={detailBusy} onClick={() => setEditor(editorForResource(selected))}><Pencil />编辑</button>
                {selected.asset_id ? <button type="button" disabled={detailBusy} onClick={() => void openFile()}><ExternalLink />打开</button> : null}
              </div>
              <div>
                <button type="button" disabled={detailBusy} onClick={() => void toggleArchive()}><Archive />{selected.archived_at ? '恢复' : '归档'}</button>
                {deleteConfirm ? <button className="is-danger" type="button" disabled={detailBusy} onClick={() => void removeSelected()}><Trash2 />确认删除</button> : <button type="button" disabled={detailBusy} onClick={() => setDeleteConfirm(true)}><Trash2 />删除</button>}
              </div>
            </footer>
          </section>
        </div>
      ) : null}

      {editor ? (
        <div className="resource-library-layer">
          <button className="resource-library-backdrop" type="button" aria-label="关闭编辑" onClick={() => setEditor(null)} />
          <section className="resource-library-editor" role="dialog" aria-modal="true" aria-labelledby="resource-library-editor-title">
            <header><div><small>{RESOURCE_META[editor.type].label}</small><h2 id="resource-library-editor-title">{editor.resource ? '编辑资料' : '添加资料'}</h2></div><button type="button" aria-label="关闭" onClick={() => setEditor(null)}><X /></button></header>
            {editor.type === 'file' && !editor.resource ? (
              <div className="resource-library-file-picker">
                <input ref={fileInputRef} type="file" accept=".pdf,.txt,.md,application/pdf,text/plain,text/markdown" onChange={(event) => {
                  const file = event.target.files?.[0] ?? null
                  setEditor((current) => current ? { ...current, file, title: current.title || file?.name.replace(/\.[^.]+$/, '') || '' } : current)
                }} />
                <button type="button" onClick={() => fileInputRef.current?.click()}><Upload /><span><strong>{editor.file?.name ?? '选择文件'}</strong><small>{editor.file ? `${Math.max(1, Math.round(editor.file.size / 1024))} KB` : 'PDF、TXT、Markdown，最大 10MB'}</small></span></button>
              </div>
            ) : null}
            <label><span>标题</span><input value={editor.title} maxLength={300} placeholder="给资料一个清楚的名称" onChange={(event) => setEditor({ ...editor, title: event.target.value })} /></label>
            {editor.type === 'link' ? <label><span>网页链接</span><input type="url" value={editor.url} placeholder="https://example.com/article" onChange={(event) => setEditor({ ...editor, url: event.target.value })} /></label> : null}
            {editor.type !== 'file' ? <label><span>{editor.type === 'note' ? '正文' : '备注（可选）'}</span><textarea autoFocus={editor.type === 'note'} rows={7} maxLength={200000} value={editor.content} placeholder={editor.type === 'note' ? '写下希望 Ripple 可以引用的内容。' : '补充这份网页资料的用途或摘要。'} onChange={(event) => setEditor({ ...editor, content: event.target.value })} /></label> : null}
            {!editor.resource && projectItems.length > 0 ? <label><span>保存范围</span><select value={editor.projectId} onChange={(event) => setEditor({ ...editor, projectId: event.target.value })}><option value="">个人资料</option>{projectItems.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}</select></label> : null}
            {editorError ? <p className="resource-library-editor-error" role="alert">{editorError}</p> : null}
            <footer><button type="button" disabled={editorBusy} onClick={() => setEditor(null)}>取消</button><button className="is-primary" type="button" disabled={editorBusy} onClick={() => void saveEditor()}>{editorBusy ? '保存中…' : '保存资料'}</button></footer>
          </section>
        </div>
      ) : null}
    </section>
  )
}
