import {
  ArrowLeft,
  CalendarDays,
  Check,
  ChevronRight,
  Clock3,
  ListTodo,
  LoaderCircle,
  Mic,
  Plus,
  RefreshCw,
  Video,
} from 'lucide-react'
import type { MeetingDetail, MeetingRecord } from '../api'

function durationLabel(seconds: number | null) {
  if (seconds === null) return '时长计算中'
  const minutes = Math.floor(seconds / 60)
  const rest = seconds % 60
  return `${minutes}:${String(rest).padStart(2, '0')}`
}

function timeLabel(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp * 1000))
}

function meetingTitle(meeting: MeetingRecord) {
  if (meeting.title.trim()) return meeting.title
  if (meeting.status === 'recording') return '正在记录'
  if (meeting.status === 'failed') return '会议整理失败'
  return '正在整理会议'
}

function MeetingStatus({ status }: { status: MeetingRecord['status'] }) {
  if (status === 'ready') return <span className="meeting-status is-ready"><Check />已完成</span>
  if (status === 'failed') return <span className="meeting-status is-failed">整理失败</span>
  return <span className="meeting-status is-processing"><LoaderCircle />{status === 'recording' ? '记录中' : '整理中'}</span>
}

export function MeetingRecords({
  items,
  detail,
  busy,
  error,
  onBack,
  onOpen,
  onCloseDetail,
  onRetry,
  onPromoteAction,
  onStart,
}: {
  items: MeetingRecord[]
  detail: MeetingDetail | null
  busy: boolean
  error: string
  onBack(): void
  onOpen(id: string): void
  onCloseDetail(): void
  onRetry(id: string): void
  onPromoteAction(meetingId: string, actionId: string): void
  onStart(): void
}) {
  if (detail) {
    return (
      <section className="meeting-screen meeting-detail-screen">
        <header className="meeting-screen-header">
          <button type="button" aria-label="返回会议记录" onClick={onCloseDetail}><ArrowLeft /></button>
          <div><small>MEETING / {detail.status.toUpperCase()}</small><h1>{meetingTitle(detail)}</h1></div>
          <span />
        </header>

        <main className="meeting-detail-content">
          <div className="meeting-detail-meta">
            <span>{detail.mode === 'video' ? <Video /> : <Mic />}{detail.mode === 'video' ? '视频' : '音频'}</span>
            <span><CalendarDays />{timeLabel(detail.started_at)}</span>
            <span><Clock3 />{durationLabel(detail.duration_seconds)}</span>
          </div>

          {detail.status === 'processing' || detail.status === 'recording' ? (
            <section className="meeting-processing-panel" aria-live="polite">
              <LoaderCircle />
              <h2>正在整理这次会议</h2>
              <p>逐字稿已经保存，标题、摘要和行动项生成后会自动出现。</p>
            </section>
          ) : null}

          {detail.status === 'failed' ? (
            <section className="meeting-processing-panel is-failed">
              <h2>整理没有完成</h2>
              <p>{detail.last_error || '可以重新生成会议标题、摘要和行动项。'}</p>
              <button type="button" onClick={() => onRetry(detail.id)}><RefreshCw />重新生成</button>
            </section>
          ) : null}

          {detail.summary ? (
            <section className="meeting-detail-section">
              <header><small>SUMMARY</small><h2>会议摘要</h2></header>
              <p className="meeting-summary-copy">{detail.summary}</p>
            </section>
          ) : null}

          {detail.action_items.length > 0 ? (
            <section className="meeting-detail-section">
              <header><small>ACTIONS / {detail.action_items.length}</small><h2>行动项</h2></header>
              <div className="meeting-actions-list">
                {detail.action_items.map((action) => (
                  <article key={action.id}>
                    <ListTodo />
                    <span>{action.title}</span>
                    {action.todo_id ? (
                      <strong><Check />已加入</strong>
                    ) : (
                      <button type="button" onClick={() => onPromoteAction(detail.id, action.id)}>加入待办</button>
                    )}
                  </article>
                ))}
              </div>
            </section>
          ) : null}

          <section className="meeting-detail-section meeting-transcript-section">
            <header><small>TRANSCRIPT / {detail.transcript.length}</small><h2>逐字稿</h2></header>
            {detail.transcript.length > 0 ? (
              <div className="meeting-transcript-list">
                {detail.transcript.map((segment) => (
                  <article key={segment.id}>
                    <strong>{segment.role === 'user' ? '我' : 'Ripple'}</strong>
                    <p>{segment.content}</p>
                    <time>{new Date(segment.created_at * 1000).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })}</time>
                  </article>
                ))}
              </div>
            ) : (
              <p className="meeting-empty-copy">本次会议没有识别到逐字稿。</p>
            )}
          </section>
        </main>
      </section>
    )
  }

  return (
    <section className="meeting-screen">
      <header className="meeting-screen-header">
        <button type="button" aria-label="返回首页" onClick={onBack}><ArrowLeft /></button>
        <div><small>RECORDS / {items.length}</small><h1>会议记录</h1></div>
        <span />
      </header>
      <main className="meeting-list-content">
        <button className="meeting-start-button" type="button" onClick={onStart}>
          <span><Mic /></span>
          <span>
            <strong>开始会议记录</strong>
            <small>只记录和整理，Ripple 不会回答</small>
          </span>
          <Plus />
        </button>
        {busy ? <div className="meeting-list-loading"><LoaderCircle /><span>读取会议记录</span></div> : null}
        {error ? <div className="meeting-list-error">{error}</div> : null}
        {!busy && !error && items.length === 0 ? (
          <div className="meeting-list-empty">
            <CalendarDays />
            <small>MEETING / EMPTY</small>
            <h2>还没有会议记录</h2>
            <p>开始一次独立会议记录，结束后会自动生成标题、摘要和行动项。</p>
          </div>
        ) : null}
        <div className="meeting-record-list">
          {items.map((meeting) => (
            <button type="button" key={meeting.id} onClick={() => onOpen(meeting.id)}>
              <span className="meeting-record-icon">{meeting.mode === 'video' ? <Video /> : <Mic />}</span>
              <span className="meeting-record-copy">
                <small>{timeLabel(meeting.started_at)} · {durationLabel(meeting.duration_seconds)}</small>
                <strong>{meetingTitle(meeting)}</strong>
                <span>{meeting.summary || (meeting.status === 'failed' ? '点击查看并重新生成' : '逐字稿已保存，正在生成摘要')}</span>
                <MeetingStatus status={meeting.status} />
              </span>
              <ChevronRight />
            </button>
          ))}
        </div>
      </main>
    </section>
  )
}
