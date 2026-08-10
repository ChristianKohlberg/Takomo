// The left navigation rail: brand, the five surfaces, and the profile block.
//
// This replaces the horizontal nav strip that used to live inside AppHeader.
// The strip had to scroll sideways the moment a fifth surface arrived, which
// hid whatever did not fit behind an edge nothing signals — and every surface
// name competed with the project picker and the action buttons for the same
// row. A rail gives the surfaces their own axis, so adding a sixth costs
// vertical space nobody is short of.
//
// Two states, one toggle: EXPANDED shows icon + label, COLLAPSED shows icons
// only. The choice is the caller's to persist (`useNavCollapsed`), because it
// is a viewer preference, not page state.
//
// On a phone the expanded rail would eat 224px of a 375px viewport, so there it
// OVERLAYS the content with a backdrop instead of pushing it, and a spacer keeps
// the collapsed strip's place in the flow. That is a structural difference, not
// a visual one, which is why it uses `useIsPhone` rather than a `md:` prefix.
import type { ReactNode } from 'react'
import {
  CalendarClockIcon,
  InboxIcon,
  LayoutGridIcon,
  LightbulbIcon,
  LogOutIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  SettingsIcon,
} from 'lucide-react'
import { Logo } from './Logo'
import { ProjectPicker, type ProjectOption, type ProjectPickerLabels } from './ProjectPicker'
import { cn } from '@/lib/utils'
import { useIsPhone } from '@/hooks/useIsPhone'

export interface NavLabels {
  board: string
  inbox: string
  initiatives: string
  schedules: string
  settings: string
}

export interface NavRailLabels {
  /** Tooltip on the toggle when the rail is collapsed. */
  expand: string
  /** …and when it is expanded. */
  collapse: string
  signOut: string
  /** Heading over the profile block; also the fallback when there is no actor. */
  account: string
}

export interface NavRailProps {
  nav: NavLabels
  /** Which surface is current — highlighted, and not a link to itself. */
  current: keyof NavLabels
  /**
   * A count beside a nav entry, the way /inbox badges open questions and
   * /schedules badges proposals waiting on a human. Zero renders nothing —
   * a "0" badge is noise, not information.
   */
  badges?: Partial<Record<keyof NavLabels, number>>
  labels: NavRailLabels
  /**
   * The project scope, which every surface reads. Omitted entirely when there is
   * nothing to pick — /settings is about the token, not a project.
   */
  projects?: ProjectOption[]
  project?: string
  onProject?: (id: string) => void
  projectLabels?: ProjectPickerLabels
  collapsed: boolean
  onCollapsed: (collapsed: boolean) => void
  /** Who the token belongs to, from `/v1/whoami`. Omitted when unknown. */
  actor?: string
  /** The scopes that token carries; the first meaningful one is shown as a role. */
  scopes?: string[]
  onSignOut: () => void
  /**
   * Client-side navigation, when the rail is mounted inside a router.
   *
   * The rail renders real `<a href>` anchors either way — middle-click,
   * cmd-click and "copy link" have to keep working, and a bare `<button>` would
   * break all three. This only intercepts the plain left-click. Omit it and the
   * anchors navigate normally, which is what lets this component render
   * standalone in a design-system preview, where there is no router to call.
   */
  onNavigate?: (href: string) => void
}

const NAV_HREF: Record<keyof NavLabels, string> = {
  board: '/board',
  inbox: '/inbox',
  initiatives: '/initiatives',
  schedules: '/schedules',
  settings: '/settings',
}

const NAV_ICON: Record<keyof NavLabels, typeof LayoutGridIcon> = {
  board: LayoutGridIcon,
  inbox: InboxIcon,
  initiatives: LightbulbIcon,
  schedules: CalendarClockIcon,
  settings: SettingsIcon,
}

const NAV_ORDER: (keyof NavLabels)[] = ['board', 'inbox', 'initiatives', 'schedules', 'settings']

/** The role worth showing, most privileged first. Anything else reads as "agent". */
function roleOf(scopes: string[] | undefined): string {
  if (!scopes || scopes.length === 0) return ''
  if (scopes.includes('admin')) return 'admin'
  if (scopes.includes('human')) return 'human'
  if (scopes.some((s) => s.startsWith('expert:'))) return 'expert'
  if (scopes.includes('write')) return 'agent'
  return 'read-only'
}

/** A modified click means "open this somewhere else"; hijacking it is the classic SPA regression. */
function plainLeftClick(e: React.MouseEvent): boolean {
  return !e.defaultPrevented && e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="text-muted-foreground hover:text-primary hover:bg-muted flex size-8 flex-none cursor-pointer items-center justify-center rounded-lg"
    >
      {children}
    </button>
  )
}

export function NavRail({
  nav,
  current,
  badges,
  labels,
  projects,
  project = '',
  onProject,
  projectLabels,
  collapsed,
  onCollapsed,
  actor,
  scopes,
  onSignOut,
  onNavigate,
}: NavRailProps) {
  const isPhone = useIsPhone()
  const expanded = !collapsed
  // Expanded-on-a-phone is the only case that cannot share the viewport.
  const overlay = isPhone && expanded
  const role = roleOf(scopes)
  const initial = (actor || labels.account).trim().charAt(0).toUpperCase()

  return (
    <>
      {overlay && (
        <>
          {/* Keeps the content from sliding left as the rail lifts out of flow. */}
          <div className="w-14 flex-none" aria-hidden />
          <div
            className="fixed inset-0 z-40 bg-black/40"
            aria-hidden
            onClick={() => onCollapsed(true)}
          />
        </>
      )}

      <aside
        className={cn(
          // NOT `overflow-y-auto` on the aside: an overflow container clips on
          // BOTH axes, and the project picker's popover is wider than the rail.
          // It rendered half off-screen until the scrolling moved to the nav
          // list, which is the only part long enough to need it anyway.
          'bg-card border-r-border-soft flex flex-none flex-col border-r',
          expanded ? 'w-56' : 'w-14',
          overlay && 'fixed inset-y-0 left-0 z-50 shadow-[var(--shadow)]',
        )}
      >
        <div
          className={cn(
            'flex min-h-[58px] flex-none items-center gap-2 px-3 py-2.5',
            collapsed && 'justify-center',
          )}
        >
          {expanded && (
            <div className="flex min-w-0 items-center gap-2.5 text-[color:var(--accent2)]">
              <Logo />
              <span className="text-foreground truncate text-base font-[750] tracking-[-0.02em]">
                takomo
              </span>
            </div>
          )}
          <span className={cn(expanded && 'grow')} />
          <IconButton
            label={collapsed ? labels.expand : labels.collapse}
            onClick={() => onCollapsed(!collapsed)}
          >
            {collapsed ? <PanelLeftOpenIcon size={18} /> : <PanelLeftCloseIcon size={18} />}
          </IconButton>
        </div>

        {/* The scope sits ABOVE the destinations, because it changes what each
            of them shows — reading it after picking a surface would be reading
            the qualifier after the noun. */}
        {projects && projectLabels && (
          <div className={cn('flex-none px-2 pb-1', collapsed && 'flex justify-center px-0')}>
            <ProjectPicker
              projects={projects}
              value={project}
              onChange={(id) => onProject?.(id)}
              labels={projectLabels}
              collapsed={collapsed}
            />
          </div>
        )}

        <nav className="flex min-h-0 grow flex-col gap-1 overflow-y-auto px-2 py-2">
          {NAV_ORDER.map((key) => {
            const Icon = NAV_ICON[key]
            const count = badges?.[key] ?? 0
            const isCurrent = key === current
            const inner = (
              <>
                <span className="relative flex size-5 flex-none items-center justify-center">
                  <Icon size={18} />
                  {/* Collapsed, there is nowhere to put a number, so the badge
                      becomes a dot: "something is waiting" is the part that
                      survives losing the label. */}
                  {collapsed && count > 0 && (
                    <span className="bg-primary absolute -top-0.5 -right-1 size-2 rounded-full" />
                  )}
                </span>
                {expanded && <span className="min-w-0 grow truncate">{nav[key]}</span>}
                {expanded && count > 0 && (
                  <span className="bg-primary text-primary-foreground inline-block min-w-[17px] flex-none rounded-[9px] px-1.25 text-center text-[11px] leading-[17px] font-bold">
                    {count}
                  </span>
                )}
              </>
            )
            const cls = cn(
              'flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13px] font-[650] no-underline',
              collapsed && 'justify-center px-0',
              isCurrent
                ? 'text-primary bg-secondary font-[680]'
                : 'text-muted-foreground hover:text-primary hover:bg-muted cursor-pointer',
            )
            return isCurrent ? (
              <span key={key} className={cls} aria-current="page" title={nav[key]}>
                {inner}
              </span>
            ) : (
              <a
                key={key}
                href={NAV_HREF[key]}
                className={cls}
                title={nav[key]}
                onClick={(e) => {
                  if (!onNavigate) return
                  if (!plainLeftClick(e)) return
                  e.preventDefault()
                  // On a phone the rail is covering the page it just navigated
                  // to, so leaving it open would hide the destination.
                  if (overlay) onCollapsed(true)
                  onNavigate(NAV_HREF[key])
                }}
              >
                {inner}
              </a>
            )
          })}
        </nav>

        {/* The profile block. Sign-out used to be one more icon button in a row
            of icon buttons on every page's header, where it sat beside
            "refresh" — two adjacent glyphs, one harmless and one that ends the
            session. Down here it is the only thing in its own region. */}
        <div
          className={cn(
            'border-t-border-soft flex flex-none items-center gap-2 border-t px-2 py-2.5',
            collapsed && 'flex-col gap-1.5 px-0',
          )}
        >
          <span
            title={actor || labels.account}
            className="bg-secondary text-secondary-foreground flex size-8 flex-none items-center justify-center rounded-full text-[12.5px] font-bold"
          >
            {initial}
          </span>
          {expanded && (
            <div className="min-w-0 grow leading-tight">
              <div className="text-foreground truncate text-[12.5px] font-[650]">
                {actor || labels.account}
              </div>
              {role && (
                <div className="text-muted-foreground truncate text-[11px]">{role}</div>
              )}
            </div>
          )}
          <IconButton label={labels.signOut} onClick={onSignOut}>
            <LogOutIcon size={17} />
          </IconButton>
        </div>
      </aside>
    </>
  )
}
