// The shared header: the surface's title, the project picker, actions, and the
// DE/EN toggle.
//
// It used to carry the brand and the cross-surface nav too. Both moved into
// NavRail — the brand because it belongs with the nav, the nav because five
// surface names, a project picker and up to four action buttons do not fit one
// row and the strip was already scrolling sideways to hide the overflow. What
// is left here is what is ABOUT THE CURRENT SURFACE, which is why the title
// replaced the nav rather than being added beside it.
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import type { Locale } from '@/lib/i18n'

export interface AppHeaderProps {
  /** The current surface's name — what the nav pill used to say. */
  title: string
  lang: Locale
  onLang: (l: Locale) => void
  /** Project picker; omitted entirely when there is nothing to pick. */
  projects?: { id: string; label?: string }[]
  project?: string
  onProject?: (id: string) => void
  allProjectsLabel?: string
  projectLabel?: string
  /** Right-hand actions (a primary button, icon buttons). */
  children?: ReactNode
}

export function AppHeader({
  title,
  lang,
  onLang,
  projects,
  project = '',
  onProject,
  allProjectsLabel,
  projectLabel = 'project',
  children,
}: AppHeaderProps) {
  return (
    <header className="bg-card border-b-border-soft flex min-h-[58px] flex-none flex-wrap items-center gap-3 border-b px-5 py-2.5">
      <h1 className="text-foreground min-w-0 truncate text-base font-[750] tracking-[-0.02em]">
        {title}
      </h1>

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
