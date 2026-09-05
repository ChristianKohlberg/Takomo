// Shared page header with persistent project and Inbox navigation.
import type { ReactNode } from 'react'
import { AppNavigation } from './AppNavigation'
import { cn } from '@/lib/utils'
import type { Locale } from '@/lib/i18n'

export interface AppHeaderProps {
  /** The current surface's name — what the nav pill used to say. */
  title: string
  subtitle?: string
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
  subtitle,
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
    <div className="flex min-w-0 flex-wrap items-center gap-2 sm:gap-3">
      <AppNavigation />
      <div className="order-3 w-full min-w-0 md:order-none md:w-auto md:flex-1"><h1 className="text-foreground truncate text-base font-[750] tracking-[-0.02em]">{title}</h1>{subtitle && <p className="truncate text-sm text-muted-foreground">{subtitle}</p>}</div>
      <div className="ml-auto md:ml-0">{languages}</div>
    </div>
    <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
      {views}
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">{children}</div>
    </div>
  </header>
  return <header className="bg-card border-b-border-soft flex min-h-[58px] flex-none flex-wrap items-center gap-2 border-b px-2 py-2.5 sm:gap-3 sm:px-5">
    <AppNavigation />
    <h1 className="text-foreground min-w-0 flex-1 truncate text-base font-[750] tracking-[-0.02em]">{title}</h1>
    {children}
    {languages}
  </header>
}
