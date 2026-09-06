// The left navigation rail: project scope and Inbox (or the brand), the work
// surfaces, and the profile block.
//
// This replaces the horizontal nav strip that used to live inside AppHeader.
// The strip had to scroll sideways the moment a fifth surface arrived, which
// hid whatever did not fit behind an edge nothing signals — and every surface
// name competed with the project picker and the action buttons for the same
// row. A rail gives the surfaces their own axis.
//
// Settings is not a fifth surface — it is account/token configuration, reached
// from the profile block's menu rather than beside Board and Inbox. Sign-out
// lives there too; the standalone icon beside the avatar was folded in so the
// block is one control, not avatar-plus-glyph.
//
// Two states, one toggle: EXPANDED shows icon + label, COLLAPSED shows icons
// only. The choice is the caller's to persist (`useNavCollapsed`), because it
// is a viewer preference, not page state.
//
// On a phone the expanded rail would eat 224px of a 375px viewport, so there it
// OVERLAYS the content with a backdrop instead of pushing it, and a spacer keeps
// the collapsed strip's place in the flow. That is a structural difference, not
// a visual one, which is why it uses `useIsPhone` rather than a `md:` prefix.
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import {
  CalendarClockIcon,
  ListChecksIcon,
  LanguagesIcon,
  LayersIcon,
  Rows3Icon,
  InboxIcon,
  LayoutGridIcon,
  LightbulbIcon,
  LogOutIcon,
  NetworkIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  ServerIcon,
  SettingsIcon,
} from 'lucide-react'
import { Logo } from './Logo'
import { AppNavigation } from './AppNavigation'
import type { Locale } from '@/lib/i18n'
import { ProjectPicker, type ProjectOption, type ProjectPickerLabels } from './ProjectPicker'
import { cn } from '@/lib/utils'
import { useIsPhone } from '@/hooks/useIsPhone'
import { Hint } from '@/components/Hint'

export interface NavLabels {
  lanes?: string
  epics: string
  board: string
  inbox: string
  specification: string
  initiatives: string
  schedules: string
  environments: string
  agentQueues?: string
}

export interface NavRailLabels {
  /** Tooltip on the toggle when the rail is collapsed. */
  expand: string
  /** …and when it is expanded. */
  collapse: string
  signOut: string
  /** Heading over the profile block; also the fallback when there is no actor. */
  account: string
  /** Profile-menu entry for /settings. */
  settings: string
}

export interface NavRailProps {
  lang?: Locale
  onLang?: (lang: Locale) => void
  /** Set by AppShell: project scope and Inbox take the top row in place of the
   *  brand, the collapse toggle drops beneath it, and Inbox leaves the list. */
  navigationInHeader?: boolean
  nav: NavLabels
  /**
   * Which surface is current — highlighted, and not a link to itself — or
   * `account` on /settings, which highlights the profile trigger instead.
   */
  current: keyof NavLabels | 'account'
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
  epics: '/epics',
  lanes: '/lanes',
  inbox: '/inbox',
  specification: '/specification',
  initiatives: '/initiatives',
  schedules: '/schedules',
  environments: '/environments',
  agentQueues: '/agent-queues',
}

const NAV_ICON: Record<keyof NavLabels, typeof LayoutGridIcon> = {
  board: LayoutGridIcon,
  epics: LayersIcon,
  lanes: Rows3Icon,
  inbox: InboxIcon,
  specification: NetworkIcon,
  initiatives: LightbulbIcon,
  schedules: CalendarClockIcon,
  environments: ServerIcon,
  agentQueues: ListChecksIcon,
}

// Specification is FIRST, and it is one entry rather than three.
//
// The map, the written plan and the tests are three renderings of one thing —
// the same nodes drawn, composed as prose, and checked — so they are one place
// you go and a switch you flip, not three destinations to choose between. The
// rail's job is to say what parts of the product exist; picking a view of one
// part is the surface's own business, and it lives in that surface's header.
//
// Everything after it is ordered as before: the work surfaces, then the idea
// being nurtured, then scheduling, then Environments, which is configuration —
// you read what needs verifying far more often than you edit where it runs.
const NAV_ORDER: (keyof NavLabels)[] = [
  'specification',
  'board',
  'lanes',
  'epics',
  'inbox',
  'initiatives',
  'schedules',
  'environments',
  'agentQueues',
]

const SETTINGS_HREF = '/settings'

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
    <Hint text={label}>
      <button
        type="button"
        aria-label={label}
        onClick={onClick}
        className="text-muted-foreground hover:text-primary hover:bg-muted flex size-8 flex-none cursor-pointer items-center justify-center rounded-lg"
      >
        {children}
      </button>
    </Hint>
  )
}

export function NavRail({
  navigationInHeader = false,
  lang,
  onLang,
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
  const accountActive = current === 'account'
  const accountName = actor || labels.account

  const [menuOpen, setMenuOpen] = useState(false)
  const [menuActive, setMenuActive] = useState(0)
  const menuRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const settingsRef = useRef<HTMLAnchorElement>(null)
  const languageRef = useRef<HTMLButtonElement>(null)
  const signOutRef = useRef<HTMLButtonElement>(null)
  const menuId = useId()
  const settingsItemId = `${menuId}-settings`
  const signOutItemId = `${menuId}-signout`

  useEffect(() => {
    if (!menuOpen) return
    setMenuActive(0)
    settingsRef.current?.focus()
  }, [menuOpen])

  useEffect(() => {
    function onDoc(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [])

  function closeMenu() {
    setMenuOpen(false)
    triggerRef.current?.focus()
  }

  function goSettings(e: React.MouseEvent<HTMLAnchorElement>) {
    if (!onNavigate) return
    if (!plainLeftClick(e)) return
    e.preventDefault()
    if (overlay) onCollapsed(true)
    setMenuOpen(false)
    onNavigate(SETTINGS_HREF)
  }

  function signOut() {
    setMenuOpen(false)
    onSignOut()
  }

  function onTriggerKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      setMenuOpen(true)
    }
  }

  function onMenuKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      const next = Math.min(menuActive + 1, onLang ? 2 : 1)
      setMenuActive(next)
      if (next === 0) settingsRef.current?.focus()
      else if (next === 1) signOutRef.current?.focus()
      else languageRef.current?.focus()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      const next = Math.max(menuActive - 1, 0)
      setMenuActive(next)
      if (next === 0) settingsRef.current?.focus()
      else if (next === 1) signOutRef.current?.focus()
      else languageRef.current?.focus()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      closeMenu()
    }
  }

  return (
    <>
      {overlay && (
        <>
          {/* Keeps the content from sliding left as the rail lifts out of flow. */}
          <div className={navigationInHeader ? "w-24 flex-none" : "w-14 flex-none"} aria-hidden />
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
          expanded ? 'w-56' : navigationInHeader ? 'w-24' : 'w-14',
          overlay && 'fixed inset-y-0 left-0 z-50 shadow-[var(--shadow)]',
        )}
      >
        <div
          className={cn(
            'flex min-h-[58px] flex-none items-center gap-2 px-3 py-2.5',
            collapsed && 'justify-center',
            navigationInHeader && collapsed && 'px-1',
          )}
        >
          {navigationInHeader ? <AppNavigation navigation={{ nav, current, badges, labels, projects, project, onProject, projectLabels, collapsed, onCollapsed, onSignOut, onNavigate }} /> : expanded && (
            <div className="flex min-w-0 items-center gap-2.5 text-[color:var(--accent2)]">
              <Logo />
              <span className="text-foreground truncate text-base font-[750] tracking-[-0.02em]">
                takomo
              </span>
            </div>
          )}

          {!navigationInHeader && <span className={cn(expanded && 'grow')} />}
          {(!navigationInHeader) && <IconButton
            label={collapsed ? labels.expand : labels.collapse}
            onClick={() => onCollapsed(!collapsed)}
          >
            {collapsed ? <PanelLeftOpenIcon size={18} /> : <PanelLeftCloseIcon size={18} />}
          </IconButton>}
        </div>

        {navigationInHeader && <div className="flex justify-end px-2">
          <IconButton label={collapsed ? labels.expand : labels.collapse} onClick={() => onCollapsed(!collapsed)}>
            {collapsed ? <PanelLeftOpenIcon size={18} /> : <PanelLeftCloseIcon size={18} />}
          </IconButton>
        </div>}

        {/* The scope sits ABOVE the destinations, because it changes what each
            of them shows — reading it after picking a surface would be reading
            the qualifier after the noun. */}
        {!navigationInHeader && projects && projectLabels && (
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
          {NAV_ORDER.filter((key) => !navigationInHeader || key !== 'inbox').map((key) => {
            const label = nav[key] ?? (key === 'lanes' ? 'Lanes' : lang === 'de' ? 'Agenten-Queue' : 'Agent queue')
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
                {expanded && <span className="min-w-0 grow truncate">{label}</span>}
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
              <Hint key={key} text={label}>
                <span className={cls} aria-current="page" aria-label={label}>
                  {inner}
                </span>
              </Hint>
            ) : (
              <Hint key={key} text={label}>
                <a
                  href={NAV_HREF[key]}
                  className={cls}
                  aria-label={label}
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
              </Hint>
            )
          })}
        </nav>

        {/* Profile block at the bottom: avatar opens a menu for settings and
            sign-out. Hand-rolled rather than Radix — ProjectPicker already owns
            the popover-placement pattern for a collapsed rail, and Settings must
            stay a real `<a href>` for middle-click and copy-link. */}
        <div
          className={cn(
            'border-t-border-soft flex flex-none border-t px-2 py-2.5',
            collapsed && 'justify-center px-0',
          )}
        >
          <div ref={menuRef} className={cn('relative', collapsed && 'flex justify-center')}>
            <Hint text={accountName}>
              <button
                ref={triggerRef}
                type="button"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                aria-controls={menuId}
                aria-label={accountName}
                onClick={() => setMenuOpen((o) => !o)}
                onKeyDown={onTriggerKeyDown}
                className={cn(
                  'hover:bg-muted flex w-full cursor-pointer items-center gap-2 rounded-lg px-0 py-0 text-left',
                  collapsed && 'justify-center',
                  accountActive && 'text-primary bg-secondary',
                )}
              >
                <span
                  className="bg-secondary text-secondary-foreground flex size-8 flex-none items-center justify-center rounded-full text-[12.5px] font-bold"
                >
                  {initial}
                </span>
                {expanded && (
                  <div className="min-w-0 grow leading-tight">
                    <div className="text-foreground truncate text-[12.5px] font-[650]">
                      {accountName}
                    </div>
                    {role && (
                      <div className="text-muted-foreground truncate text-[11px]">{role}</div>
                    )}
                  </div>
                )}
              </button>
            </Hint>

            {menuOpen && (
              <div
                id={menuId}
                role="menu"
                aria-label={labels.account}
                onKeyDown={onMenuKeyDown}
                className={cn(
                  'bg-card border-border absolute z-50 min-w-44 overflow-hidden rounded-lg border py-1 shadow-[var(--shadow)]',
                  // Collapsed, beside the trigger — under it would hang off the
                  // 56px strip. Expanded, above — the block sits on the bottom edge.
                  collapsed ? 'top-0 left-full ml-1' : 'bottom-full left-0 mb-1',
                )}
              >
                <a
                  ref={settingsRef}
                  id={settingsItemId}
                  role="menuitem"
                  href={SETTINGS_HREF}
                  className={cn(
                    'text-foreground hover:bg-muted flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-[13px] font-[650] no-underline',
                    menuActive === 0 && 'bg-accent',
                    accountActive && 'text-primary font-[680]',
                  )}
                  onClick={goSettings}
                >
                  <SettingsIcon size={16} className="flex-none" />
                  <span>{labels.settings}</span>
                </a>
                <button
                  ref={signOutRef}
                  id={signOutItemId}
                  type="button"
                  role="menuitem"
                  className={cn(
                    'text-foreground hover:bg-muted flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left text-[13px] font-[650]',
                    menuActive === 1 && 'bg-accent',
                  )}
                  onClick={signOut}
                >
                  <LogOutIcon size={16} className="flex-none" />
                  <span>{labels.signOut}</span>
                </button>
                {lang && onLang && <button
                  ref={languageRef}
                  type="button"
                  role="menuitem"
                  onClick={() => onLang(lang === 'de' ? 'en' : 'de')}
                  className={cn('text-foreground hover:bg-muted flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left text-[13px] font-[650]', menuActive === 2 && 'bg-accent')}
                >
                  <LanguagesIcon size={16} className="flex-none" />
                  <span>{lang === 'de' ? 'Sprache: Deutsch → English' : 'Language: English → Deutsch'}</span>
                </button>}
              </div>
            )}
          </div>
        </div>
      </aside>
    </>
  )
}
