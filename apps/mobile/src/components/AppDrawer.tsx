import {
  Brain,
  CalendarDays,
  ChevronRight,
  Database,
  FolderKanban,
  History,
  ListTodo,
  Settings,
  UserRoundCog,
  X,
  type LucideIcon,
} from 'lucide-react'
import { useEffect } from 'react'
import { UserAvatar } from './UserAvatar'

export type AppDestination =
  | 'home'
  | 'history'
  | 'meetings'
  | 'projects'
  | 'materials'
  | 'memories'
  | 'todos'
  | 'personalization'
  | 'settings'

const destinationGroups: Array<{
  label: string
  items: Array<{ destination: AppDestination; label: string; description: string; icon: LucideIcon }>
}> = [
  {
    label: '记录',
    items: [
      { destination: 'history', label: '对话记录', description: '查看与 AI 的历史对话', icon: History },
      { destination: 'meetings', label: '会议记录', description: '查看与整理会议内容', icon: CalendarDays },
    ],
  },
  {
    label: '组织',
    items: [
      { destination: 'projects', label: '项目', description: '管理项目与进展', icon: FolderKanban },
      { destination: 'materials', label: '资料库', description: '存储与管理重要资料', icon: Database },
    ],
  },
  {
    label: '助手',
    items: [
      { destination: 'memories', label: '记忆', description: '查看与管理助手记忆', icon: Brain },
      { destination: 'todos', label: '待办', description: '管理你的待办事项', icon: ListTodo },
    ],
  },
  {
    label: '账户',
    items: [
      { destination: 'personalization', label: '个性化', description: '定制你的使用体验', icon: UserRoundCog },
      { destination: 'settings', label: '设置', description: '管理应用设置与偏好', icon: Settings },
    ],
  },
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
        aria-label="关闭菜单"
        onClick={onClose}
      />
      <aside className="app-drawer" role="dialog" aria-modal="true" aria-label="菜单">
        <header className="app-drawer-header">
          <div>
            <h1>菜单</h1>
            <p>所有内容与工具</p>
          </div>
          <button type="button" aria-label="关闭菜单" onClick={onClose}>
            <X aria-hidden="true" />
          </button>
        </header>

        <nav className="drawer-navigation" aria-label="Ripple 功能菜单">
          {destinationGroups.map((group) => (
            <section className="drawer-group" key={group.label} aria-labelledby={`drawer-${group.label}`}>
              <h2 id={`drawer-${group.label}`}>{group.label}</h2>
              <div className="drawer-group-list">
                {group.items.map(({ destination, label, description, icon: Icon }) => (
                  <button
                    className={active === destination ? 'is-active' : ''}
                    key={destination}
                    type="button"
                    aria-current={active === destination ? 'page' : undefined}
                    onClick={() => onSelect(destination)}
                  >
                    <span className="drawer-item-icon"><Icon aria-hidden="true" /></span>
                    <span className="drawer-item-copy">
                      <strong>{label}</strong>
                      <small>{description}</small>
                    </span>
                    <ChevronRight aria-hidden="true" />
                  </button>
                ))}
              </div>
            </section>
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
