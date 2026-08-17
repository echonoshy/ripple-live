import {
  Images,
  ListChecks,
  MessageCircleMore,
  UserRound,
  type LucideIcon,
} from 'lucide-react'

export type AppTab = 'chat' | 'memories' | 'todos' | 'profile'

const items: Array<{ tab: AppTab; label: string; icon: LucideIcon }> = [
  { tab: 'chat', label: '对话', icon: MessageCircleMore },
  { tab: 'memories', label: '记忆', icon: Images },
  { tab: 'todos', label: '待办', icon: ListChecks },
  { tab: 'profile', label: '我的', icon: UserRound },
]

export function BottomNav({
  active,
  onSelect,
}: {
  active: AppTab
  onSelect(tab: AppTab): void
}) {
  return (
    <nav className="bottom-nav" aria-label="主导航">
      <div className="bottom-nav-items">
        {items.map(({ tab, label, icon: IconComponent }) => (
          <button
            className={active === tab ? 'is-active' : ''}
            key={tab}
            type="button"
            aria-current={active === tab ? 'page' : undefined}
            aria-label={label}
            onClick={() => onSelect(tab)}
          >
            <IconComponent aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
      </div>
    </nav>
  )
}
