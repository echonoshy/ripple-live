import { Archive, CheckSquare, DotsThreeVertical, MagnifyingGlass, PushPin, X } from '@phosphor-icons/react'
import { useState } from 'react'
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

const historyScopes: Array<{ value: LibraryView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'pinned', label: '已置顶' },
  { value: 'archived', label: '已归档' },
]

const memoryScopes: Array<{ value: LibraryView; label: string }> = [
  { value: 'all', label: '全部' },
  { value: 'pinned', label: '置顶' },
  { value: 'images', label: '图片' },
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
  const [memoryMenuOpen, setMemoryMenuOpen] = useState(false)
  const searchId = kind === '聊天历史' ? 'history-search' : 'memory-search'
  const compactHistory = kind === '聊天历史'
  const scopes = compactHistory ? historyScopes : memoryScopes

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

  const manageButton = (
    <button
      className="library-manage-button"
      type="button"
      aria-label={`管理${kind}`}
      onClick={() => {
        setMemoryMenuOpen(false)
        onStartSelection()
      }}
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
            onClick={() => {
              setMemoryMenuOpen(false)
              onScopeChange(item.value)
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      {!compactHistory && (
        <div className="library-toolbar-meta">
          <p>
            {scope === 'archived'
              ? '正在查看已归档记忆，可在管理模式中恢复。'
              : '图片筛选只显示有保存画面的记忆。'}
          </p>
          <div className="library-toolbar-actions">
            {manageButton}
            <div className="library-overflow">
              <button
                className="library-overflow-button"
                type="button"
                aria-label="更多记忆管理"
                aria-expanded={memoryMenuOpen}
                onClick={() => setMemoryMenuOpen((open) => !open)}
              >
                <DotsThreeVertical aria-hidden="true" />
              </button>
              {memoryMenuOpen && (
                <div className="library-overflow-menu" role="menu">
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      if (scope === 'archived') onScopeChange('all')
                      else onScopeChange('archived')
                      setMemoryMenuOpen(false)
                    }}
                  >
                    <Archive aria-hidden="true" />
                    {scope === 'archived' ? '返回全部记忆' : '查看已归档'}
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
