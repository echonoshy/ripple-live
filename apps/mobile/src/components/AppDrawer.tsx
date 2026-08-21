import {
  Brain,
  History,
  MessageCircle,
  Settings,
  SquareCheckBig,
  X,
  type LucideIcon,
} from 'lucide-react'
import { useEffect } from 'react'
import { UserAvatar } from './UserAvatar'

export type AppDestination =
  | 'home'
  | 'history'
  | 'memories'
  | 'todos'
  | 'settings'

const destinations: Array<{
  destination: AppDestination
  label: string
  icon: LucideIcon
}> = [
  { destination: 'home', label: '开始', icon: MessageCircle },
  { destination: 'history', label: '对话历史', icon: History },
  { destination: 'memories', label: '记忆', icon: Brain },
  { destination: 'todos', label: '待办', icon: SquareCheckBig },
  { destination: 'settings', label: '设置', icon: Settings },
]

export function AppDrawer({
  open,
  active,
  accountLabel,
  avatarUrl,
  server,
  token,
  onClose,
  onSelect,
}: {
  open: boolean
  active: AppDestination
  accountLabel: string
  avatarUrl: string | null
  server: string
  token: string
  onClose(): void
  onSelect(destination: AppDestination): void
}) {
  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [onClose, open])

  if (!open) return null

  return (
    <div className="app-drawer-layer">
      <button
        className="app-drawer-backdrop"
        type="button"
        aria-label="关闭导航"
        onClick={onClose}
      />
      <aside className="app-drawer" role="dialog" aria-modal="true" aria-label="主导航">
        <header className="app-drawer-header">
          <strong><span aria-hidden="true" />Ripple</strong>
          <button type="button" aria-label="关闭导航" onClick={onClose}>
            <X aria-hidden="true" />
          </button>
        </header>

        <nav className="drawer-navigation" aria-label="Ripple 功能">
          {destinations.map(({ destination, label, icon: Icon }) => (
            <button
              className={active === destination ? 'is-active' : ''}
              key={destination}
              type="button"
              aria-current={active === destination ? 'page' : undefined}
              onClick={() => onSelect(destination)}
            >
              <Icon aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
        </nav>

        <footer className="app-drawer-account">
          <UserAvatar
            server={server}
            token={token}
            email={accountLabel}
            avatarUrl={avatarUrl}
          />
          <div>
            <strong>{accountLabel}</strong>
            <small>Ripple Live</small>
          </div>
        </footer>
      </aside>
    </div>
  )
}
