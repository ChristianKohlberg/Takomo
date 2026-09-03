// The left rail of `/documents` — the plan's own outline.
//
// Every row is a section: a number, a dot whose size reads its level, a title
// that quiets as it goes deeper, and a fold. The number is the load-bearing
// part. It is not decoration and not a table of contents: it is the shared
// address of a part of the plan, which is what lets somebody say "§2.1 promises
// something §4 takes back" and be understood without quoting either.
//
// Depth is carried by three things at once — indent, dot size, and type weight —
// because any one of them alone makes a deep tree a counting exercise. The rule
// borrowed from the design it follows is that depth never shouts: a level-3
// heading is quieter than a level-1, never louder.
//
// What changed with one plan seen two ways: the sections are NODES now, not
// folders holding documents. The rail used to fold a folder and the document
// that named it into one row, because writing a map up produced exactly that
// pair — and with the conversion gone there is no pair, just the tree the canvas
// draws, read as reading order.
//
// Props-only, as everything in the barrel is: the fold lives in the page (it is
// a browser-local viewer preference), the sections come from `lib/plan-sections`,
// and this file decides nothing.
import type { PlanSection } from '@/lib/plan-sections'
import { sectionCount, visibleSections } from '@/lib/plan-sections'
import { pendingInSubtree } from '@/lib/plan-proposals'
import type { Standing } from '@/lib/plan-trace'
import { cn } from '@/lib/utils'

export interface OutlineRailLabels {
  expand: string
  collapse: string
  /** What a folded section is holding. `{n}` is the count, at any depth. */
  folded: string
  /** A section nobody has given a title yet. */
  untitled: string
  /** Read out beside the mark. */
  standingConfirmed: string
  standingChanged: string
  standingUnseen: string
  /** What the ◆ beside a row means. `{n}` is how many are waiting. */
  pending: string
}

export interface OutlineRailProps {
  sections: PlanSection[]
  /** The section the column is scrolled to, if any. */
  selected: string | null
  onSelect: (key: string) => void
  /** Section keys this viewer has folded. Never shared. */
  collapsed: ReadonlySet<string>
  onToggle: (key: string) => void
  /** Where each section stands, by key. Absent means the plan has no history yet. */
  standing?: Readonly<Record<string, Standing>>
  /** Proposals waiting on a person, by section key. A FOLDED row reports what
   *  is waiting beneath it too — folding a branch is not a decision to stop
   *  caring what an agent offered inside it. */
  pending?: Readonly<Record<string, number>>
  labels: OutlineRailLabels
  className?: string
}

/** Dot diameter in px by depth. Inline because it is data, not a style — a
 *  class per level would be five classes saying one thing. */
const DOT = [7, 5.5, 4.5, 4]

const dotSize = (depth: number): number => DOT[Math.min(depth, DOT.length - 1)] as number

/** Type quiets with depth, and stops quieting: past level three the indent and
 *  the dot carry it, and shrinking further would just be unreadable. */
const titleClass = (depth: number): string =>
  depth === 0
    ? 'text-[13.5px] font-[620] text-foreground'
    : depth === 1
      ? 'text-[13px] font-medium text-foreground'
      : 'text-[12.5px] text-muted-foreground'

const dotClass = (depth: number, active: boolean): string =>
  active
    ? 'bg-primary'
    : depth === 0
      ? 'bg-foreground/70'
      : depth === 1
        ? 'bg-muted-foreground/70'
        : 'bg-muted-foreground/40'

/** One character per standing. A glyph is a reminder for somebody who already
 *  knows; the accessible name is always the sentence. */
const MARK: Record<Standing, string> = {
  confirmed: '✓',
  changed: '~',
  unseen: '·',
}

const markClass: Record<Standing, string> = {
  confirmed: 'text-emerald-600 dark:text-emerald-400',
  changed: 'text-amber-600 dark:text-amber-400',
  unseen: 'text-muted-foreground/60',
}

export function OutlineRail({
  sections,
  selected,
  onSelect,
  collapsed,
  onToggle,
  standing,
  pending,
  labels,
  className,
}: OutlineRailProps) {
  const rows = visibleSections(sections, collapsed)
  const standingLabel: Record<Standing, string> = {
    confirmed: labels.standingConfirmed,
    changed: labels.standingChanged,
    unseen: labels.standingUnseen,
  }

  return (
    <ul className={cn('flex flex-col', className)}>
      {rows.map((section) => {
        const active = section.key === selected
        const hasChildren = section.children.length > 0
        const folded = hasChildren && collapsed.has(section.key)
        const hidden = folded ? sectionCount(section) : 0
        const stands = standing?.[section.key]
        const waiting = pending
          ? folded
            ? pendingInSubtree(section, pending)
            : (pending[section.key] ?? 0)
          : 0
        return (
          <li
            key={section.key}
            className={cn(
              'group flex items-center gap-1 rounded-md',
              active ? 'bg-accent' : 'hover:bg-accent/50',
            )}
            // Indentation is data: a depth-4 row needs a depth-4 inset, and
            // Tailwind cannot spell an arbitrary one without a class per level.
            style={{ paddingLeft: `${2 + section.depth * 12}px` }}
          >
            {hasChildren ? (
              <button
                type="button"
                aria-label={folded ? labels.expand : labels.collapse}
                aria-expanded={!folded}
                onClick={() => onToggle(section.key)}
                className="text-muted-foreground hover:text-foreground w-4 flex-none text-[10px]"
              >
                {folded ? '▸' : '▾'}
              </button>
            ) : (
              <span className="w-4 flex-none" aria-hidden="true" />
            )}

            <span className="flex w-2.5 flex-none justify-center" aria-hidden="true">
              <span
                className={cn('block rounded-full', dotClass(section.depth, active))}
                style={{
                  width: `${dotSize(section.depth)}px`,
                  height: `${dotSize(section.depth)}px`,
                }}
              />
            </span>

            <button
              type="button"
              onClick={() => onSelect(section.key)}
              className="flex min-w-0 grow items-baseline gap-1.5 py-1.5 pr-1 text-left"
            >
              <span className="text-muted-foreground flex-none font-mono text-[10.5px]">
                {section.number}
              </span>
              <span
                className={cn(
                  'min-w-0 truncate',
                  titleClass(section.depth),
                  section.title ? '' : 'italic opacity-70',
                )}
              >
                {section.title || labels.untitled}
              </span>
            </button>

            {folded && (
              <span
                className="text-muted-foreground flex-none font-mono text-[10px]"
                title={labels.folded.replace('{n}', String(hidden))}
              >
                ⊞ {hidden}
              </span>
            )}
            {waiting > 0 && (
              <span
                className="flex-none px-1 font-mono text-[11px] text-amber-600 dark:text-amber-400"
                title={labels.pending.replace('{n}', String(waiting))}
              >
                <span aria-hidden="true">◆{waiting}</span>
                <span className="sr-only">{labels.pending.replace('{n}', String(waiting))}</span>
              </span>
            )}
            {stands && (
              <span
                className={cn('flex-none px-1 font-mono text-[11px]', markClass[stands])}
                title={standingLabel[stands]}
              >
                <span aria-hidden="true">{MARK[stands]}</span>
                <span className="sr-only">{standingLabel[stands]}</span>
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}
