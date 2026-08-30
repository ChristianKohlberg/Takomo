// The verbs for the selected thought, floating just above it.
//
// The idea is borrowed from the Ultraplan prototype and so is its discipline:
// affordances live in the margin and appear only on selection, so the map itself
// stays a map. What is NOT borrowed is the prototype's habit of putting every
// verb it had into the same strip — six glyphs on a dark capsule is a menu that
// has lost the argument for being a pill.
//
// Three or four, chosen by frequency, and the rest are elsewhere: `+` beside the
// node adds a child, the badge on it manages attachments, the right-click menu
// removes it, and ⌘K carries everything else. If a fifth verb ever seems to
// belong here, that is the signal it belongs in ⌘K.
import { cn } from '@/lib/utils'

export interface PillVerb {
  id: string
  /** One character. The accessible name is `label`, never this. */
  glyph: string
  label: string
}

export interface NodePillProps {
  verbs: readonly PillVerb[]
  onRun: (id: string) => void
  /** Named for the thought it acts on, so a screen reader hears which node. */
  ariaLabel: string
  className?: string
}

export function NodePill({ verbs, onRun, ariaLabel, className }: NodePillProps) {
  if (verbs.length === 0) return null
  return (
    <div
      role="toolbar"
      aria-label={ariaLabel}
      // The canvas keyboard is a map keyboard — Enter grows a branch, Space
      // folds one — and none of that may fire because somebody pressed Enter on
      // a button in here.
      onKeyDown={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      className={cn(
        'bg-foreground flex w-fit items-center gap-0.5 rounded-full px-1 py-1 shadow-md',
        className,
      )}
    >
      {verbs.map((v) => (
        <button
          key={v.id}
          type="button"
          title={v.label}
          aria-label={v.label}
          onClick={() => onRun(v.id)}
          className="text-background hover:bg-background/20 cursor-pointer rounded-full px-2 py-0.5 text-[12px] leading-none"
        >
          {v.glyph}
        </button>
      ))}
    </div>
  )
}
