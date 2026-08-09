// The shared header: brand, cross-surface nav, project picker, actions, and the
// DE/EN toggle.
//
// Every page carried its own copy of this markup. It takes props now, which is
// what lets the next three ports reuse it instead of forking it a fourth time.
import type { ReactNode } from 'react'
import { Logo } from './Logo'
import { cn } from '@/lib/utils'
import type { Locale } from '@/lib/i18n'

export interface NavLabels {
  board: string
  inbox: string
  initiatives: string
  schedules: string
}

export interface AppHeaderProps {
  nav: NavLabels
  /** Which surface is current — rendered as a pill instead of a link. */
  current: keyof NavLabels
  lang: Locale
  onLang: (l: Locale) => void
  /** Project picker; omitted entirely when there is nothing to pick. */
  projects?: { id: string; label?: string }[]
  project?: string
  onProject?: (id: string) => void
  allProjectsLabel?: string
  projectLabel?: string
  /**
   * A count beside a nav entry, the way /inbox badges open questions and
   * /schedules badges proposals waiting on a human. Zero renders nothing —
   * a "0" badge is noise, not information.
   */
  badges?: Partial<Record<keyof NavLabels, number>>
  /** Right-hand actions (a primary button, icon buttons). */
  children?: ReactNode
  /**
   * Client-side navigation, when the header is mounted inside a router.
   *
   * The nav renders real `<a href>` anchors either way — middle-click, cmd-click
   * and "copy link" have to keep working, and a bare `<button>` would break all
   * three. This only intercepts the plain left-click. Omit it and the anchors
   * navigate normally, which is what lets this component render standalone in a
   * design-system preview, where there is no router to call.
   */
  onNavigate?: (href: string) => void
}

const NAV_HREF: Record<keyof NavLabels, string> = {
  board: '/board',
  inbox: '/inbox',
  initiatives: '/initiatives',
  schedules: '/schedules',
}

const linkCls =
  'text-muted-foreground hover:text-primary hover:bg-muted cursor-pointer rounded-lg px-3.5 py-1.5 text-base md:text-[13px] font-[650] no-underline'

export function AppHeader({
  nav,
  current,
  lang,
  onLang,
  projects,
  project = '',
  onProject,
  allProjectsLabel,
  projectLabel = 'project',
  badges,
  children,
  onNavigate,
}: AppHeaderProps) {
  return (
    <header className="bg-card border-b-border-soft flex min-h-[58px] flex-none flex-wrap items-center gap-3 border-b px-5 py-2.5">
      <div className="flex items-center gap-2.5 text-[color:var(--accent2)]">
        <Logo />
        <span className="text-foreground text-base font-[750] tracking-[-0.02em]">takomo</span>
      </div>

      {/* Scrollable rather than wrapping: four surface names do not fit a 375px
          row, and wrapping them pushes everything below further down the screen
          on the surface that can least afford it. */}
      <nav className="ml-1 flex min-w-0 max-w-full gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {(Object.keys(nav) as (keyof NavLabels)[]).map((key) => {
          const count = badges?.[key] ?? 0
          const badge =
            count > 0 ? (
              <span className="bg-primary text-primary-foreground ml-1.5 inline-block min-w-[17px] rounded-[9px] px-1.25 text-center text-[11px] leading-[17px] font-bold">
                {count}
              </span>
            ) : null
          return key === current ? (
            <span key={key} className={cn(linkCls, 'shrink-0 text-primary bg-secondary font-[680]')}>
              {nav[key]}
              {badge}
            </span>
          ) : (
            <a
              key={key}
              href={NAV_HREF[key]}
              className={cn(linkCls, 'shrink-0')}
              onClick={(e) => {
                // Let the browser handle anything that is not a plain left-click:
                // a modified click means "open this somewhere else", and
                // hijacking it is the classic SPA regression.
                if (!onNavigate) return
                if (e.defaultPrevented) return
                if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
                e.preventDefault()
                onNavigate(NAV_HREF[key])
              }}
            >
              {nav[key]}
              {badge}
            </a>
          )
        })}
      </nav>

      {projects && (
        // A native select rather than a Radix one, on purpose: it is one control
        // per page, it behaves correctly on mobile for free, and a portalled
        // listbox would add weight to all four documents for no gain here.
        <select
          aria-label={projectLabel}
          title={projectLabel}
          value={project}
          onChange={(e) => onProject?.(e.target.value)}
          className="bg-muted text-foreground border-border hover:border-ring hover:text-primary max-w-55 cursor-pointer appearance-none rounded-lg border px-3 py-1.5 text-[13px] font-[650]"
        >
          {allProjectsLabel != null && <option value="">{allProjectsLabel}</option>}
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label ?? p.id}
            </option>
          ))}
        </select>
      )}

      <span className="grow" />
      {children}

      <div className="text-muted-foreground inline-flex gap-0.5 text-[11.5px] font-bold">
        {(['de', 'en'] as const).map((l) => (
          <span
            key={l}
            role="button"
            tabIndex={0}
            onClick={() => onLang(l)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onLang(l)
              }
            }}
            className={cn(
              'cursor-pointer rounded-md px-1.5 py-1',
              lang === l && 'bg-secondary text-secondary-foreground',
            )}
          >
            {l.toUpperCase()}
          </span>
        ))}
      </div>
    </header>
  )
}
