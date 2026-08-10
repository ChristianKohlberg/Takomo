// The frame the console is laid out in: a section rail and a titled panel.
//
// The first version was four Cards in one column. Everything was equally
// prominent, which meant nothing was — "download the entire database" sat at the
// same weight as a list of project names, and the page grew a scrollbar before
// it had any real content in it. Sections that are SWITCHED rather than stacked
// give each one the whole panel, and make the page's shape legible at a glance
// instead of by scrolling it.
//
// Switched, not anchor-scrolled: scroll-spy needs an observer per section and
// still lies at the end of the page, where the last section can never reach the
// top. There are four destinations here. A rail is enough.
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface SectionDef<K extends string> {
  key: K
  label: string
  /** Rendered under the label in the rail — what the section is FOR. */
  hint: string
}

export interface SectionNavProps<K extends string> {
  sections: readonly SectionDef<K>[]
  current: K
  onSelect: (key: K) => void
}

/**
 * The rail: a sidebar from `md` up, a scrolling row of chips below it.
 *
 * The two are one list rendered twice rather than a single flex that reflows,
 * because they want genuinely different content — the sidebar has room for the
 * hint line and the chip row does not, and squeezing hints into chips is what
 * made the mobile board unreadable before #129.
 */
export function SectionNav<K extends string>({
  sections,
  current,
  onSelect,
}: SectionNavProps<K>) {
  return (
    <>
      {/* Mobile: chips. Scrolls rather than wraps, so the panel below keeps its
          vertical space on a phone. */}
      {/* `shrink-0` is load-bearing: this sits in a `flex-col` whose panel
          sibling is `flex-1`, so without it the chips are compressed to a few
          pixels of blue and the only way to change section on a phone is gone. */}
      <nav className="flex shrink-0 gap-1.5 overflow-x-auto pb-1 md:hidden [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {sections.map((s) => (
          <button
            key={s.key}
            type="button"
            onClick={() => onSelect(s.key)}
            aria-current={s.key === current ? 'page' : undefined}
            className={cn(
              'shrink-0 cursor-pointer rounded-lg px-3 py-1.5 text-[13px] font-[650] transition-colors',
              s.key === current
                ? 'bg-secondary text-primary'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground',
            )}
          >
            {s.label}
          </button>
        ))}
      </nav>

      {/* Desktop: a sidebar with the hint line. */}
      <nav className="hidden w-52 shrink-0 flex-col gap-0.5 md:flex">
        {sections.map((s) => (
          <button
            key={s.key}
            type="button"
            onClick={() => onSelect(s.key)}
            aria-current={s.key === current ? 'page' : undefined}
            className={cn(
              'cursor-pointer rounded-lg px-3 py-2 text-left transition-colors',
              s.key === current ? 'bg-secondary' : 'hover:bg-muted',
            )}
          >
            <div
              className={cn(
                'text-[13px] font-[650]',
                s.key === current ? 'text-primary' : 'text-foreground',
              )}
            >
              {s.label}
            </div>
            <div className="text-muted-foreground mt-0.5 text-[11.5px] leading-snug">{s.hint}</div>
          </button>
        ))}
      </nav>
    </>
  )
}

export interface SectionProps {
  title: string
  description?: string
  /** Right-aligned in the header — the section's primary action. */
  action?: ReactNode
  children: ReactNode
}

/**
 * One panel: a title, what it is for, its primary action, and its content.
 *
 * The action belongs in the HEADER rather than beside the rows it affects.
 * "+ New token" floating above a list reads as an action on the list; in the
 * header it reads as an action on the section, which is what it is.
 */
export function Section({ title, description, action, children }: SectionProps) {
  return (
    <section className="flex min-w-0 flex-col gap-4">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-foreground text-[15px] font-[750] tracking-[-0.01em]">{title}</h2>
          {description && (
            <p className="text-muted-foreground mt-1 max-w-prose text-[13px] leading-relaxed">
              {description}
            </p>
          )}
        </div>
        {action}
      </header>
      {children}
    </section>
  )
}

/**
 * A labelled row of facts — the shape "actor / scopes / projects" wants.
 *
 * A definition list rather than a table: there is one subject (this token), so
 * the rows are its properties, not records to compare.
 */
export function FactRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="border-b-border-soft flex flex-wrap items-baseline gap-x-4 gap-y-1 border-b py-2.5 last:border-b-0">
      <dt className="text-muted-foreground w-24 shrink-0 text-[10.5px] font-bold tracking-[0.05em] uppercase">
        {label}
      </dt>
      <dd className="min-w-0 flex-1 text-[13px]">{children}</dd>
    </div>
  )
}

/**
 * What a section shows when it has nothing to show.
 *
 * Not a bare sentence: an empty list and a failed fetch look identical when both
 * render as one grey line, and the box makes "this is the content" explicit.
 */
export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div className="border-border-soft text-muted-foreground rounded-xl border border-dashed px-4 py-8 text-center text-[13px]">
      {children}
    </div>
  )
}
