import { createContext, useContext } from 'react'
import { InboxIcon, PanelLeftCloseIcon, PanelLeftOpenIcon } from 'lucide-react'
import type { NavRailProps } from './NavRail'
import { ProjectPicker } from './ProjectPicker'
import { Hint } from './Hint'
import { cn } from '@/lib/utils'

export const AppNavigationContext = createContext<NavRailProps | null>(null)

/** Global controls stay in the header, outside the scrolling navigation list. */
export function AppNavigation() {
  const rail = useContext(AppNavigationContext)
  if (!rail) return null
  const count = rail.badges?.inbox ?? 0
  const active = rail.current === 'inbox'
  const toggleLabel = rail.collapsed ? rail.labels.expand : rail.labels.collapse
  const ToggleIcon = rail.collapsed ? PanelLeftOpenIcon : PanelLeftCloseIcon

  return (
    <div className="flex flex-none items-center gap-2">
      <Hint text={toggleLabel}>
        <button
          type="button"
          aria-label={toggleLabel}
          onClick={() => rail.onCollapsed(!rail.collapsed)}
          className="text-muted-foreground hover:text-primary hover:bg-muted flex size-10 items-center justify-center rounded-lg"
        >
          <ToggleIcon size={18} />
        </button>
      </Hint>
      {rail.projects && rail.projectLabels && (
        <ProjectPicker
          projects={rail.projects}
          value={rail.project ?? ''}
          onChange={(id) => rail.onProject?.(id)}
          labels={rail.projectLabels}
          header
        />
      )}
      <Hint text={rail.nav.inbox}>
        <a
          href="/inbox"
          aria-label={rail.nav.inbox}
          aria-current={active ? 'page' : undefined}
          className={cn(
            'relative flex size-10 items-center justify-center rounded-lg',
            active ? 'text-primary bg-secondary' : 'text-muted-foreground hover:text-primary hover:bg-muted',
          )}
          onClick={(event) => {
            if (!rail.onNavigate || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
            event.preventDefault()
            rail.onNavigate('/inbox')
          }}
        >
          <InboxIcon size={18} />
          {count > 0 && (
            <span className="bg-primary text-primary-foreground absolute -top-0.5 -right-1 min-w-[17px] rounded-full px-1 text-center text-[10px] leading-[17px] font-bold">
              {count > 99 ? '99+' : count}
            </span>
          )}
        </a>
      </Hint>
    </div>
  )
}
