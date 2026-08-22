import {
  Archive,
  ArrowLeft,
  ChevronRight,
  FolderClock,
  LoaderCircle,
  Mic,
  MoreVertical,
  Pencil,
  RotateCcw,
  Video,
} from 'lucide-react'
import { useState } from 'react'
import type { ConversationSummary, ProjectRecord } from '../../api'

function conversationTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp * 1000))
}

export function ProjectDetailScreen({
  project,
  conversations,
  busy,
  callBusy,
  error,
  onBack,
  onEdit,
  onArchive,
  onRestore,
  onStartAudio,
  onStartVideo,
  onOpenConversation,
  onRetry,
}: {
  project: ProjectRecord
  conversations: ConversationSummary[]
  busy: boolean
  callBusy: boolean
  error: string
  onBack(): void
  onEdit(): void
  onArchive(): void
  onRestore(): void
  onStartAudio(): void
  onStartVideo(): void
  onOpenConversation(conversation: ConversationSummary): void
  onRetry(): void
}) {
  const [menuOpen, setMenuOpen] = useState(false)
  const archived = project.archived_at !== null

  return (
    <section className="project-screen project-detail-screen">
      <header className="project-screen-header project-detail-header">
        <button type="button" aria-label="返回项目列表" onClick={onBack}><ArrowLeft /></button>
        <div><h1>{project.name}</h1><small>PROJECT / {archived ? 'ARCHIVED' : 'ACTIVE'}</small></div>
        <div className="project-detail-menu-wrap">
          <button type="button" aria-label="项目操作" aria-expanded={menuOpen} onClick={() => setMenuOpen((value) => !value)}><MoreVertical /></button>
          {menuOpen ? (
            <div className="project-detail-menu">
              <button type="button" onClick={() => { setMenuOpen(false); onEdit() }}><Pencil />编辑项目</button>
              {archived ? (
                <button type="button" onClick={() => { setMenuOpen(false); onRestore() }}><RotateCcw />恢复项目</button>
              ) : (
                <button type="button" onClick={() => { setMenuOpen(false); onArchive() }}><Archive />归档项目</button>
              )}
            </div>
          ) : null}
        </div>
      </header>

      <main className="project-detail-content">
        <section className="project-context-panel">
          <header><span>&lt;/&gt;</span><small>PROJECT.KIRO</small><i aria-hidden="true" /></header>
          <div className="project-context-line">
            <small>01</small><div><strong>项目说明</strong><p>{project.description || '暂无项目说明'}</p></div>
          </div>
          <div className="project-context-line">
            <small>02</small><div><strong>固定规则</strong><p>{project.instructions || '暂无固定规则'}</p></div>
          </div>
        </section>

        {archived ? (
          <div className="project-archived-notice">
            <Archive /><span><strong>项目已归档</strong><small>恢复后才能继续发起项目对话。</small></span>
            <button type="button" onClick={onRestore}>恢复</button>
          </div>
        ) : (
          <div className="project-call-actions">
            <button type="button" disabled={callBusy} onClick={onStartAudio}><Mic />{callBusy ? '正在准备' : '开始语音'}</button>
            <button type="button" disabled={callBusy} onClick={onStartVideo}><Video />视频聊聊</button>
          </div>
        )}

        <section className="project-conversations-section">
          <header><div><small>CONVERSATIONS / {conversations.length}</small><h2>项目对话</h2></div></header>
          {busy ? (
            <div className="project-loading"><LoaderCircle /><span>读取项目对话</span></div>
          ) : error ? (
            <div className="project-error"><p>{error}</p><button type="button" onClick={onRetry}>重新加载</button></div>
          ) : conversations.length === 0 ? (
            <div className="project-conversations-empty"><FolderClock /><strong>还没有项目对话</strong><p>{archived ? '这个项目归档前没有留下对话记录。' : '从上方开始语音或视频，第一段对话会沉淀在这里。'}</p></div>
          ) : (
            <div className="project-conversation-list">
              {conversations.map((conversation) => (
                <button type="button" key={conversation.id} onClick={() => onOpenConversation(conversation)}>
                  <span className="project-conversation-icon" aria-hidden="true">&gt;_</span>
                  <span>
                    <strong>{conversation.title || '新对话'}</strong>
                    <small>{conversation.preview || '这次对话还没有文本内容'}</small>
                  </span>
                  <time>{conversationTime(conversation.updated_at)}</time>
                  <ChevronRight aria-hidden="true" />
                </button>
              ))}
            </div>
          )}
        </section>
      </main>
    </section>
  )
}
