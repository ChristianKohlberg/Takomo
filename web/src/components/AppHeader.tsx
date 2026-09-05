// The shared header: the surface's title, its actions, and the DE/EN toggle.
//
// Three things have left it. The brand and the cross-surface nav went to
// NavRail, because five surface names plus a picker plus four action buttons do
// not fit one row and the strip was already scrolling sideways to hide the
// overflow. The project picker followed them, because it is not about the
// current surface — it SCOPES all of them, and a control every page obeys
// belongs with the navigation rather than in each page's own toolbar.
//
// What is left is what is ABOUT THE CURRENT SURFACE. That is the line to apply
// when deciding whether something new goes here or in the rail.
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import type { Locale } from '@/lib/i18n'

export interface AppHeaderProps {
  /** The current surface's name — what the nav pill used to say. */
  title: string
  lang: Locale
  onLang: (l: Locale) => void
  /** Right-hand actions (a primary button, icon buttons). */
  children?: ReactNode
  /**
   * Views of THIS surface, beside its name — the specification's map, plan and
   * tests. Beside the title rather than out with the actions on the right,
   * because it names what you are looking at rather than doing something to it.
   */
  views?: ReactNode
}

export function AppHeader({
  title,
  lang,
  onLang,
  children,
  views,
}: AppHeaderProps) {
  const languages = <div className="text-muted-foreground inline-flex gap-0.5 text-[11.5px] font-bold">
    {(['de', 'en'] as const).map((l) => <button key={l} type="button" aria-pressed={lang === l}
      onClick={() => onLang(l)}
      className={cn('cursor-pointer rounded-md px-2 py-1.5', lang === l && 'bg-secondary text-secondary-foreground')}>
      {l.toUpperCase()}
    </button>)}
  </div>
  if (views) return <header className="bg-card border-b-border-soft flex flex-none flex-col gap-2 border-b px-3 py-3 sm:px-5">
    <div className="flex min-w-0 items-center gap-3">
      <h1 className="text-foreground min-w-0 flex-1 truncate text-base font-[750] tracking-[-0.02em]">{title}</h1>
      {languages}
    </div>
    <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
      {views}
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">{children}</div>
    </div>
  </header>
  return <header className="bg-card border-b-border-soft flex min-h-[58px] flex-none flex-wrap items-center gap-3 border-b px-5 py-2.5">
    <h1 className="text-foreground min-w-0 truncate text-base font-[750] tracking-[-0.02em]">{title}</h1>
    <span className="grow" />
    {children}
    {languages}
  </header>
}
