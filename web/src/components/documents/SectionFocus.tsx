// Section focus chrome: the band that says the document is narrowed (and how to
// widen it), and the breadcrumb from the document down to the focused section.
// Pure presentation; the page owns the state, which lives in the URL (`focus=`).

import { X } from 'lucide-react'

export interface SectionFocusBandProps {
  /** The landmark's accessible name, e.g. "Section focus". */
  label: string
  /** "Focus: 7 Audit plan". */
  text: string
  /** The exit button's text, e.g. "Show all sections". */
  exit: string
  /** Its tooltip, naming the Escape shortcut. */
  exitHint?: string
  /** "3 proposals outside" and the like; empty entries are not shown. */
  outside?: string[]
  onExit: () => void
}

export function SectionFocusBand({ label, text, exit, exitHint, outside = [], onExit }: SectionFocusBandProps) {
  return (
    <section
      aria-label={label}
      className="section-focus-band sticky top-0 z-30 flex flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-sky-200 bg-sky-50 px-4 py-1.5 text-sm text-sky-950 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-100"
    >
      <span className="min-w-0 truncate font-medium">{text}</span>
      {outside.length > 0 && (
        <span className="text-xs text-sky-800 dark:text-sky-300">{outside.join(' · ')}</span>
      )}
      <button
        type="button"
        onClick={onExit}
        title={exitHint}
        aria-keyshortcuts="Escape"
        className="ml-auto inline-flex min-h-8 shrink-0 items-center gap-1 rounded px-2 hover:bg-sky-100 dark:hover:bg-sky-900"
      >
        <X className="size-3.5" aria-hidden="true" />
        {exit}
      </button>
    </section>
  )
}

export interface SectionFocusBreadcrumbProps {
  label: string
  /** The document itself; choosing it leaves focus. */
  root: string
  ancestors: { key: string; label: string }[]
  current: string
  onRoot: () => void
  /** Focus an ancestor instead. */
  onAncestor: (key: string) => void
}

export function SectionFocusBreadcrumb({ label, root, ancestors, current, onRoot, onAncestor }: SectionFocusBreadcrumbProps) {
  const crumb = 'min-h-8 max-w-full truncate rounded px-1 text-left underline-offset-2 hover:bg-muted hover:underline'
  return (
    <nav aria-label={label} className="section-focus-breadcrumb text-muted-foreground mb-2 pt-1 text-sm">
      <ol className="flex flex-wrap items-center gap-x-1">
        <li className="flex min-w-0 items-center">
          <button type="button" className={crumb} onClick={onRoot}>{root}</button>
        </li>
        {ancestors.map(ancestor => (
          <li key={ancestor.key} className="flex min-w-0 items-center gap-x-1">
            <span aria-hidden="true">›</span>
            <button type="button" className={crumb} onClick={() => onAncestor(ancestor.key)}>{ancestor.label}</button>
          </li>
        ))}
        <li className="flex min-w-0 items-center gap-x-1">
          <span aria-hidden="true">›</span>
          <span aria-current="page" className="text-foreground truncate px-1 font-medium">{current}</span>
        </li>
      </ol>
    </nav>
  )
}
