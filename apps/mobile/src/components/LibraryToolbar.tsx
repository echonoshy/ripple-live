import { Archive, CheckSquare, MagnifyingGlass, PushPin, X } from '@phosphor-icons/react'
import type { LibraryAction, LibraryView } from '../library'

export type LibraryToolbarProps = {
  kind: '聊天历史' | '视觉记忆'
  query: string
  scope: LibraryView
  selectionCount: number
  selectionMode: boolean
  itemCount: number
  onQueryChange(value: string): void
  onScopeChange(value: LibraryView): void
  onBatchAction(action: LibraryAction): void
  onStartSelection(): void
  onSelectAll(): void
  onCancelSelection(): void
}

const scopes: Array<{ value: LibraryView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'pinned', label: '已置顶' },
  { value: 'archived', label: '已归档' },
]

export function LibraryToolbar({
  kind,
  query,
  scope,
  selectionCount,
  selectionMode,
  itemCount,
  onQueryChange,
  onScopeChange,
  onBatchAction,
  onStartSelection,
  onSelectAll,
  onCancelSelection,
}: LibraryToolbarProps) {
  if (selectionMode) {
    return (
      <div className="library-selection-bar" aria-label={`已选择 ${selectionCount} 项`}>
        <div className="library-selection-heading">
          <strong>{selectionCount > 0 ? `已选择 ${selectionCount} 项` : '选择要管理的项目'}</strong>
          <button className="text-action" type="button" onClick={onSelectAll} disabled={itemCount === 0}>
            全选
          </button>
        </div>
        <div>
          <button type="button" disabled={selectionCount === 0} onClick={() => onBatchAction(scope === 'pinned' ? 'unpin' : 'pin')}>
            <PushPin weight="fill" aria-hidden="true" />
            {scope === 'pinned' ? '取消置顶' : '置顶'}
          </button>
          <button
            type="button"
            disabled={selectionCount === 0}
            onClick={() => onBatchAction(scope === 'archived' ? 'unarchive' : 'archive')}
          >
            <Archive aria-hidden="true" />
            {scope === 'archived' ? '恢复' : '归档'}
          </button>
          <button className="danger-action" type="button" disabled={selectionCount === 0} onClick={() => onBatchAction('delete')}>
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
  const compactHistory = kind === '聊天历史'
  const manageButton = (
    <button
      className="library-manage-button"
      type="button"
      aria-label={`管理${kind}`}
      onClick={onStartSelection}
    >
      <CheckSquare aria-hidden="true" />
      <span>管理</span>
    </button>
  )

  return (
    <div className={`library-toolbar ${compactHistory ? 'is-history' : ''}`}>
      <div className="library-query-row">
        <label className="visually-hidden" htmlFor={searchId}>搜索{kind}</label>
        <div className="library-search">
          <span className="library-search-affordance" aria-hidden="true">
            <MagnifyingGlass />
          </span>
          <input
            id={searchId}
            type="search"
            value={query}
            aria-label={`搜索${kind}`}
            placeholder={compactHistory ? '搜索对话' : '搜索备注或画面内容'}
            onChange={(event) => onQueryChange(event.target.value)}
          />
        </div>
        {compactHistory && manageButton}
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
      {!compactHistory && (
        <div className="library-toolbar-meta">
          <p>
            <strong>置顶</strong>会留在最近记录并排在最前；<strong>归档</strong>会从最近记录移出，且不再作为 Agent 的联想素材。
          </p>
          {manageButton}
        </div>
      )}
    </div>
  )
}
