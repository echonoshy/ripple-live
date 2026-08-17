import { Archive, RotateCcw as ArrowCounterClockwise, SquarePen as NotePencil, Pin as PushPin, Trash2 as Trash } from 'lucide-react'
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
        <button className="library-item-action" type="button" onClick={onRename}>
          <NotePencil aria-hidden="true" /> 重命名
        </button>
      )}
      {!archived && (
        <button className="library-item-action" type="button" onClick={() => onAction(pinned ? 'unpin' : 'pin')}>
          <PushPin aria-hidden="true" />
          {pinned ? '取消置顶' : '置顶'}
        </button>
      )}
      <button className="library-item-action" type="button" onClick={() => onAction(archived ? 'unarchive' : 'archive')}>
        {archived ? <ArrowCounterClockwise aria-hidden="true" /> : <Archive aria-hidden="true" />}
        {archived ? '恢复' : '归档'}
      </button>
      <button className="library-item-action danger-action" type="button" onClick={() => onAction('delete')}>
        <Trash aria-hidden="true" /> 删除
      </button>
    </div>
  )
}
