import { CheckIcon, InboxIcon } from 'lucide-react'
import type { NavRailProps } from './NavRail'
import { ProjectPicker } from './ProjectPicker'
import { Hint } from './Hint'
import { cn } from '@/lib/utils'

/** Project scope and inbox share the top of the navigation rail. */
export function AppNavigation({ navigation: rail }: { navigation: NavRailProps }) {
  const count = rail.badges?.inbox
  const active = rail.current === 'inbox'
  return (
    <div className="flex min-w-0 flex-1 items-center gap-1">
      {rail.projects && rail.projectLabels && (
        <div className="min-w-0 flex-1"><ProjectPicker
          projects={rail.projects}
          value={rail.project ?? ''}
          onChange={(id) => rail.onProject?.(id)}
          labels={rail.projectLabels}
          collapsed={rail.collapsed}
        /></div>
      )}
      <Hint text={rail.nav.inbox}>
        <a
          href="/inbox"
          aria-label={rail.nav.inbox}
          aria-current={active ? 'page' : undefined}
          className={cn(
            'relative flex size-10 shrink-0 items-center justify-center rounded-lg',
            active ? 'text-primary bg-secondary' : 'text-muted-foreground hover:text-primary hover:bg-muted',
          )}
          onClick={(event) => {
            if (!rail.onNavigate || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
            event.preventDefault()
            rail.onNavigate('/inbox')
          }}
        >
          <InboxIcon size={18} />
          {count != null && count > 0 ? (
            <span className="bg-primary text-primary-foreground absolute -top-0.5 -right-1 min-w-[17px] rounded-full px-1 text-center text-[10px] leading-[17px] font-bold">
              {count > 99 ? '99+' : count}
            </span>
          ) : count === 0 && (
            <span role="img" aria-label={rail.nav.inbox + ': 0'} className="bg-card absolute -top-0.5 -right-0.5 rounded-full text-emerald-600">
              <CheckIcon size={13} strokeWidth={3} />
            </span>
          )}
        </a>
      </Hint>
    </div>
  )
}
