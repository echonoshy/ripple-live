import {
  CheckCircle,
  CloudSun,
  ListChecks,
  MagnifyingGlass,
  X,
} from '@phosphor-icons/react'
import type { ReactNode } from 'react'
import { openExternalUrl } from '../live/externalLinks'
import type { LiveResult } from '../realtime/toolResults'

export type LiveResultSheetProps = {
  results: LiveResult[]
  onDismiss(callId: string): void
}

function dueLabel(dueAt: number | null) {
  if (dueAt === null) return null
  const due = new Date(dueAt * 1000)
  if (!Number.isFinite(due.getTime())) return null
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(due)
}

function assertNever(result: never): never {
  throw new Error(`Unexpected live result: ${String(result)}`)
}

function ResultBody({ result }: { result: LiveResult }) {
  switch (result.kind) {
    case 'memory_receipt':
      return (
        <Receipt icon={<CheckCircle weight="fill" />} label="记忆已保存">
          {result.title}
        </Receipt>
      )
    case 'todo_receipt': {
      const due = dueLabel(result.dueAt)
      return (
        <Receipt icon={<ListChecks weight="fill" />} label="待办已创建">
          <span>{result.title}</span>
          {due && <small>{due}</small>}
        </Receipt>
      )
    }
    case 'todo_list':
      return (
        <div className="live-result-detail">
          <strong>{result.completed ? '已完成待办' : '当前待办'}</strong>
          {result.titles.length > 0 ? (
            <ul className="live-result-todos">
              {result.titles.slice(0, 5).map((title, index) => (
                <li key={`${index}-${title}`}>{title}</li>
              ))}
            </ul>
          ) : (
            <span className="live-result-empty">没有符合条件的待办</span>
          )}
        </div>
      )
    case 'search':
      return (
        <div className="live-result-detail">
          <strong className="live-result-heading">
            <MagnifyingGlass aria-hidden="true" />
            搜索结果
          </strong>
          <ul className="live-result-sources">
            {result.items.slice(0, 3).map((item) => (
              <li key={item.url}>
                <button
                  type="button"
                  aria-label={`在外部浏览器打开来源：${item.title}`}
                  onClick={() => {
                    void openExternalUrl(item.url)
                  }}
                >
                  <span>{item.title}</span>
                  <small>{item.snippet}</small>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )
    case 'weather':
      return (
        <div className="live-result-weather">
          <CloudSun aria-hidden="true" />
          <div>
            <strong>{result.location}</strong>
            <span>{result.summary}</span>
          </div>
          {result.temperature !== null && (
            <b>{new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 }).format(result.temperature)}°</b>
          )}
        </div>
      )
    case 'generic':
      return (
        <div className="live-result-generic">
          {result.status === 'success' && <CheckCircle weight="fill" aria-hidden="true" />}
          <span>{result.label}</span>
        </div>
      )
    default:
      return assertNever(result)
  }
}

function Receipt({
  icon,
  label,
  children,
}: {
  icon: ReactNode
  label: string
  children: ReactNode
}) {
  return (
    <div className="live-result-receipt">
      <span className="live-result-icon" aria-hidden="true">{icon}</span>
      <div>
        <strong>{label}</strong>
        {children}
      </div>
    </div>
  )
}

export function LiveResultSheet({ results, onDismiss }: LiveResultSheetProps) {
  if (results.length === 0) return null

  return (
    <aside
      className="live-result-sheet"
      aria-label="本轮操作结果"
      aria-live="polite"
    >
      {results.map((result) => (
        <article
          className={`live-result-card is-${result.status}`}
          key={result.callId}
        >
          <ResultBody result={result} />
          <button
            className="live-result-dismiss"
            type="button"
            aria-label={`关闭此结果：${result.kind === 'generic' ? result.label : result.kind}`}
            onClick={() => onDismiss(result.callId)}
          >
            <X weight="bold" aria-hidden="true" />
          </button>
        </article>
      ))}
    </aside>
  )
}
