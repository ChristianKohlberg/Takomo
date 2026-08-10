// A collapsible heading over one epic's questions.
//
// The inbox list is flat, which is fine at four questions and unreadable at
// forty: a reader working through "the billing epic" had to recognise its
// tickets by id. Grouped, the same list becomes a handful of headings — and
// they collapse, so the epic you are not working on today costs one row instead
// of eleven.
//
// It is a <button>, not a clickable <div>: the whole heading is the hit target,
// `aria-expanded` says which way it goes, and a keyboard reader gets it for
// free.
import { cn } from '@/lib/utils'

export interface EpicGroupHeaderProps {
  /** The epic's title, or the "no epic" label for the remainder bucket. */
  title: string
  count: number
  collapsed: boolean
  onToggle: () => void
  className?: string
}

export function EpicGroupHeader({
  title,
  count,
  collapsed,
  onToggle,
  className,
}: EpicGroupHeaderProps) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={!collapsed}
      className={cn(
        'bg-muted/60 border-b-border-soft hover:bg-muted sticky top-0 z-10 flex w-full cursor-pointer items-center gap-2 border-b px-3.5 py-2 text-left',
        className,
      )}
    >
      {/* A rotated caret rather than two glyphs: one element, and the rotation
          is the affordance that says the row is a disclosure. */}
      <span
        aria-hidden
        className={cn(
          'text-muted-foreground inline-block text-[10px] transition-transform',
          collapsed ? '' : 'rotate-90',
        )}
      >
        ▶
      </span>
      <span className="text-foreground min-w-0 grow truncate text-[12px] font-[750] tracking-[0.04em] uppercase">
        {title}
      </span>
      <span className="text-muted-foreground shrink-0 tabular-nums text-[11.5px] font-[650]">
        {count}
      </span>
    </button>
  )
}
