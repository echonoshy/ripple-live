import { Archive, ArrowCounterClockwise, PushPin, Trash } from '@phosphor-icons/react'
import type { LibraryAction } from '../library'

export type LibraryActionsProps = {
  pinned: boolean
  archived: boolean
  onAction(action: LibraryAction): void
}

export function LibraryActions({ pinned, archived, onAction }: LibraryActionsProps) {
  return (
    <div className="library-item-actions" aria-label="项目操作">
      <button type="button" onClick={() => onAction(pinned ? 'unpin' : 'pin')}>
        <PushPin weight={pinned ? 'fill' : 'regular'} aria-hidden="true" />
        {pinned ? '取消标记' : '标记'}
      </button>
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
