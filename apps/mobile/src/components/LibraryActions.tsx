import { Archive, ArrowCounterClockwise, NotePencil, PushPin, Trash } from '@phosphor-icons/react'
import type { LibraryAction } from '../library'

export type LibraryActionsProps = {
  pinned: boolean
  archived: boolean
  onAction(action: LibraryAction): void
  onRename?(): void
}

export function LibraryActions({ pinned, archived, onAction, onRename }: LibraryActionsProps) {
  return (
    <div className={`library-item-actions ${onRename ? 'has-rename' : ''} ${archived ? 'is-archived' : ''}`} aria-label="项目操作">
      {onRename && (
        <button type="button" onClick={onRename}>
          <NotePencil aria-hidden="true" /> 重命名
        </button>
      )}
      {!archived && (
        <button type="button" onClick={() => onAction(pinned ? 'unpin' : 'pin')}>
          <PushPin weight={pinned ? 'fill' : 'regular'} aria-hidden="true" />
          {pinned ? '取消置顶' : '置顶'}
        </button>
      )}
      <button type="button" onClick={() => onAction(archived ? 'unarchive' : 'archive')}>
        {archived ? <ArrowCounterClockwise aria-hidden="true" /> : <Archive aria-hidden="true" />}
        {archived ? '恢复' : '归档'}
      </button>
      <button className="danger-action" type="button" onClick={() => onAction('delete')}>
        <Trash aria-hidden="true" /> 删除
      </button>
    </div>
  )
}
