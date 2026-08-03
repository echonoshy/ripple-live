import { Archive, MagnifyingGlass, PushPin, X } from '@phosphor-icons/react'
import type { LibraryAction, LibraryView } from '../library'

export type LibraryToolbarProps = {
  kind: '聊天历史' | '视觉记忆'
  query: string
  scope: LibraryView
  selectionCount: number
  onQueryChange(value: string): void
  onScopeChange(value: LibraryView): void
  onBatchAction(action: LibraryAction): void
  onCancelSelection(): void
}

const scopes: Array<{ value: LibraryView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'pinned', label: '已标记' },
  { value: 'archived', label: '已归档' },
]

export function LibraryToolbar({
  kind,
  query,
  scope,
  selectionCount,
  onQueryChange,
  onScopeChange,
  onBatchAction,
  onCancelSelection,
}: LibraryToolbarProps) {
  if (selectionCount > 0) {
    return (
      <div className="library-selection-bar" aria-label={`已选择 ${selectionCount} 项`}>
        <strong>已选择 {selectionCount} 项</strong>
        <div>
          <button type="button" onClick={() => onBatchAction(scope === 'pinned' ? 'unpin' : 'pin')}>
            <PushPin weight="fill" aria-hidden="true" />
            {scope === 'pinned' ? '取消标记' : '标记'}
          </button>
          <button
            type="button"
            onClick={() => onBatchAction(scope === 'archived' ? 'unarchive' : 'archive')}
          >
            <Archive aria-hidden="true" />
            {scope === 'archived' ? '恢复' : '归档'}
          </button>
          <button className="danger-action" type="button" onClick={() => onBatchAction('delete')}>
            删除
          </button>
          <button className="icon-only" type="button" aria-label="取消选择" onClick={onCancelSelection}>
            <X aria-hidden="true" />
          </button>
        </div>
      </div>
    )
  }

  const searchId = kind === '聊天历史' ? 'history-search' : 'memory-search'
  return (
    <div className="library-toolbar">
      <label htmlFor={searchId}>搜索{kind}</label>
      <div className="library-search">
        <MagnifyingGlass aria-hidden="true" />
        <input
          id={searchId}
          type="search"
          value={query}
          aria-label={`搜索${kind}`}
          placeholder={kind === '聊天历史' ? '搜索标题或对话内容' : '搜索备注或画面内容'}
          onChange={(event) => onQueryChange(event.target.value)}
        />
      </div>
      <div className="library-scope-tabs" aria-label={`${kind}视图`}>
        {scopes.map((item) => (
          <button
            key={item.value}
            type="button"
            className={scope === item.value ? 'is-active' : ''}
            aria-pressed={scope === item.value}
            onClick={() => onScopeChange(item.value)}
          >
            {item.label}
          </button>
        ))}
      </div>
    </div>
  )
}
