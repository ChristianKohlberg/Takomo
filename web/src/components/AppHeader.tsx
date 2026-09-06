// Surface title, centered view controls, and page actions. Global controls live in the rail.
import type { ReactNode } from 'react'

export interface AppHeaderProps {
  /** The current surface's name — what the nav pill used to say. */
  title: string
  subtitle?: string
  /** Right-hand actions (a primary button, icon buttons). */
  children?: ReactNode
  /** Views of this surface, centered independently of the title and actions. */
  views?: ReactNode
}

export function AppHeader({ title, subtitle, children, views }: AppHeaderProps) {
  return <header className="bg-card border-b-border-soft grid min-h-[58px] flex-none grid-cols-[minmax(0,1fr)] items-center gap-2 border-b px-3 py-2.5 md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] sm:px-5">
    <div className="min-w-0">
      <h1 className="text-foreground truncate text-base font-[750] tracking-[-0.02em]">{title}</h1>
      {subtitle && <p className="text-muted-foreground truncate text-sm">{subtitle}</p>}
    </div>
    <div className="flex min-w-0 justify-center">{views}</div>
    <div className="flex min-w-0 flex-wrap items-center justify-end gap-x-3 gap-y-1">{children}</div>
  </header>
}
