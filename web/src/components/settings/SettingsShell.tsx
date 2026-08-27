// The frame the console is laid out in: a section tab strip and a titled panel.
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
// top. There are four destinations here; a tab strip is enough.
//
// TABS, along the top, rather than the left sidebar this first used. #132 gave
// the whole app a left nav rail, and two left rails side by side is one rail too
// many: the reader cannot tell which one moves them between surfaces and which
// one moves them within this page. The global rail owns the left edge, so the
// in-surface switcher goes horizontal, where its subordinate relationship to the
// rail is legible from the layout alone.
import type { ReactNode } from 'react'
import { TabsList, TabsTrigger } from '@/components/ui/tabs'

export interface SectionDef<K extends string> {
  key: K
  label: string
  /** Shown under the panel title — what the section is FOR. */
  hint: string
}

export interface SectionTabsProps<K extends string> {
  sections: readonly SectionDef<K>[]
}

/**
 * The section switcher: an underlined tab strip that scrolls rather than wraps.
 *
 * Scrolls because four labels plus their padding do not fit 320px, and wrapping
 * them would push the panel down by a whole row on the screen that can least
 * afford it. `shrink-0` is load-bearing for the same reason it was on the chips
 * this replaced: its sibling is `flex-1`, and without it flex compresses the
 * strip to a few pixels and the only way to change section is gone. `w-full`
 * likewise overrides the primitive's `w-fit`, which would let the strip shrink
 * away from the border it draws.
 *
 * Radix owns selection now, so this no longer takes `current`/`onSelect` — the
 * `<Tabs>` root in the page does. What that buys over the hand-rolled version is
 * the keyboard contract a tab strip is supposed to have and did not: arrow keys
 * move between tabs, Home/End jump to the ends, and the strip is one tab stop
 * rather than one per section. It also stops claiming `aria-current="page"`,
 * which says "this is the current PAGE" — these switch a panel, not a page, and
 * a screen reader now hears a tablist.
 */
export function SectionTabs<K extends string>({ sections }: SectionTabsProps<K>) {
  return (
    <TabsList
      variant="line"
      className="border-b-border-soft h-auto w-full shrink-0 justify-start gap-1 overflow-x-auto rounded-none border-b p-0 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      {sections.map((s) => (
        <TabsTrigger
          key={s.key}
          value={s.key}
          // The active underline sits ON the strip's own bottom border, so the
          // row keeps one continuous baseline instead of gaining a second line
          // under the selected item.
          className="shrink-0 grow-0 cursor-pointer px-3 py-2 text-[13px] font-[650] after:bottom-[-1px] data-active:text-primary data-active:after:bg-primary"
        >
          {s.label}
        </TabsTrigger>
      ))}
    </TabsList>
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
