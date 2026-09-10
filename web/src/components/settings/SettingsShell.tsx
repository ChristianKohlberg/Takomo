// The panel primitives the settings console is laid out in: a titled section,
// a fact row and an empty state. Which section is shown is the router's job
// (`SettingsLayout`), so there is no switcher here.
import type { ReactNode } from 'react'

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
