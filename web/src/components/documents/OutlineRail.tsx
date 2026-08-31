// The left rail of `/documents` — the outline of the whole undertaking rather
// than a list of files.
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
// Props-only, as everything in the barrel is: the fold lives in the page (it is
// a browser-local viewer preference), the sections come from
// `lib/document-outline`, and this file decides nothing.
import type { Doc } from '@/lib/documents'
import type { OutlineSection } from '@/lib/document-outline'
import { sectionCount, visibleSections } from '@/lib/document-outline'
import { Hint } from '@/components/Hint'
import { cn } from '@/lib/utils'

export interface OutlineRailLabels {
  expand: string
  collapse: string
  /** What a folded section is holding. `{n}` is the count, at any depth. */
  folded: string
  /** A folder no document is filed as — a group, with nothing to open. */
  group: string
  archive: string
  unarchive: string
  archived: string
  /** Proposals waiting on a document. `{n}`. */
  waiting: string
}

export interface OutlineRailProps {
  sections: OutlineSection[]
  /** The open document's id. */
  selected: string | null
  onSelect: (id: string) => void
  /** Section keys this viewer has folded. Never shared. */
  collapsed: ReadonlySet<string>
  onToggle: (key: string) => void
  /** Absent on a read-only token: nothing to offer. */
  onArchiveToggle?: (doc: Doc) => void
  /** Proposals waiting, by document id. Only the open document can be known. */
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

export function OutlineRail({
  sections,
  selected,
  onSelect,
  collapsed,
  onToggle,
  onArchiveToggle,
  pending,
  labels,
  className,
}: OutlineRailProps) {
  const rows = visibleSections(sections, collapsed)

  return (
    <ul className={cn('flex flex-col', className)}>
      {rows.map((section) => {
        const doc = section.doc
        const active = doc !== null && doc.id === selected
        const hasChildren = section.children.length > 0
        const folded = hasChildren && collapsed.has(section.key)
        const hidden = folded ? sectionCount(section) : 0
        const waiting = doc ? (pending?.[doc.id] ?? 0) : 0
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
                style={{ width: `${dotSize(section.depth)}px`, height: `${dotSize(section.depth)}px` }}
              />
            </span>

            {doc ? (
              <button
                type="button"
                onClick={() => onSelect(doc.id)}
                className="flex min-w-0 grow items-baseline gap-1.5 py-1.5 pr-1 text-left"
              >
                <span className="text-muted-foreground flex-none font-mono text-[10.5px]">
                  {section.number}
                </span>
                <span className={cn('min-w-0 truncate', titleClass(section.depth))}>
                  {section.title}
                </span>
                {doc.archived_at && (
                  <span className="text-muted-foreground flex-none text-[10.5px]">
                    ({labels.archived})
                  </span>
                )}
              </button>
            ) : (
              // A folder nothing is filed as. It groups, so it opens and closes,
              // but there is no prose behind it to read.
              <button
                type="button"
                title={labels.group}
                onClick={() => onToggle(section.key)}
                className="flex min-w-0 grow items-baseline gap-1.5 py-1.5 pr-1 text-left"
              >
                <span className="text-muted-foreground flex-none font-mono text-[10.5px]">
                  {section.number}
                </span>
                <span
                  className={cn('min-w-0 truncate italic opacity-80', titleClass(section.depth))}
                >
                  {section.title}
                </span>
              </button>
            )}

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
                className="flex-none rounded-sm bg-amber-100 px-1 text-[10px] font-bold text-amber-900 dark:bg-amber-950 dark:text-amber-200"
                title={labels.waiting.replace('{n}', String(waiting))}
              >
                {waiting}
              </span>
            )}
            {doc && onArchiveToggle && (
              <Hint text={doc.archived_at ? labels.unarchive : labels.archive}>
                <button
                  type="button"
                  aria-label={doc.archived_at ? labels.unarchive : labels.archive}
                  onClick={() => onArchiveToggle(doc)}
                  className="text-muted-foreground hover:text-foreground flex-none px-1 text-[12px] opacity-0 group-hover:opacity-100 focus:opacity-100"
                >
                  {doc.archived_at ? '↩' : '×'}
                </button>
              </Hint>
            )}
          </li>
        )
      })}
    </ul>
  )
}
