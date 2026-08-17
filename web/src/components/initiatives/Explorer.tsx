import { useMemo } from 'react'
import { ancestors, pathOf, type TreeFolder, type TreeNode } from '@/lib/initiative-tree'
import { waiting, type Initiative } from '@/lib/initiatives'
import { cn } from '@/lib/utils'

export interface ExplorerLabels {
  empty: string
  emptyHint: string
  /** Accessible name for a folder's expand/collapse control. */
  toggle: string
  unfiled: string
  /** Accessible name for the waiting badges, given a count: `(n) => string`. */
  waitingNotes: (n: number) => string
  waitingAmendments: (n: number) => string
}

export interface ExplorerProps {
  root: TreeFolder
  selectedId: string | null
  /** Folder paths currently expanded. */
  expanded: ReadonlySet<string>
  onToggle: (path: string) => void
  onSelect: (id: string) => void
  labels: ExplorerLabels
}

/**
 * The document tree.
 *
 * Folders come from `metadata.path` and exist only because a document names
 * them, so there is no empty folder to render and no folder row to keep in sync
 * with anything. Depth is expressed with indentation rather than nested
 * scrollable boxes: the tree is one flat list of rows, which is what lets it
 * stay usable on a phone, where a nested-box tree runs out of width by the third
 * level.
 */
export function Explorer({ root, selectedId, expanded, onToggle, onSelect, labels }: ExplorerProps) {
  // Flattened to rows once per render rather than recursing in JSX: a flat list
  // is what keyboard navigation and virtualization would both want later, and
  // it keeps the indentation arithmetic in one place.
  const rows = useMemo(() => flatten(root, expanded), [root, expanded])

  if (rows.length === 0) {
    return (
      <div className="px-4 py-6">
        <p className="text-foreground m-0 text-[13px] font-semibold">{labels.empty}</p>
        <p className="text-muted-foreground mt-1 mb-0 text-[12px]">{labels.emptyHint}</p>
      </div>
    )
  }

  return (
    <div role="tree" className="py-1.5">
      {rows.map((row) =>
        row.node.kind === 'folder' ? (
          <button
            key={'f:' + row.node.path}
            type="button"
            role="treeitem"
            aria-expanded={expanded.has(row.node.path)}
            aria-label={`${labels.toggle} ${row.node.name}`}
            onClick={() => onToggle((row.node as TreeFolder).path)}
            style={{ paddingLeft: indent(row.depth) }}
            className="text-foreground hover:bg-muted flex w-full cursor-pointer items-center gap-1.5 py-1 pr-3 text-left text-[13px] font-semibold"
          >
            <Chevron open={expanded.has(row.node.path)} />
            <span className="truncate">{row.node.name}</span>
            <span className="text-muted-foreground ml-auto font-mono text-[11px]">
              {row.node.count}
            </span>
          </button>
        ) : (
          <button
            key={'d:' + row.node.initiative.id}
            type="button"
            role="treeitem"
            aria-selected={row.node.initiative.id === selectedId}
            onClick={() => onSelect((row.node as { initiative: Initiative }).initiative.id)}
            style={{ paddingLeft: indent(row.depth) + 16 }}
            className={cn(
              'hover:bg-muted flex w-full cursor-pointer items-center gap-2 py-1 pr-3 text-left text-[13px]',
              row.node.initiative.id === selectedId
                ? 'bg-secondary text-secondary-foreground font-semibold'
                : 'text-foreground',
            )}
          >
            <span className="truncate">{row.node.initiative.title}</span>
            {row.node.initiative.status !== 'open' && (
              <span className="text-muted-foreground ml-auto shrink-0 text-[10.5px] tracking-[0.04em] uppercase">
                {row.node.initiative.status}
              </span>
            )}
            <Waiting initiative={row.node.initiative} labels={labels} />
          </button>
        ),
      )}
    </div>
  )
}

/**
 * What this document is waiting on, on its own row.
 *
 * The whole point of the badges: before them, an open note or an offered rewrite
 * was invisible until you opened that document and scrolled to it, so a
 * collection of thirty gave a reader no way to find the two that needed them.
 *
 * Amendments read louder than notes because they are a decision someone is
 * blocked on, where a note is a question that can wait. Both stay small enough to
 * be ignorable — a row that shouts is one people learn to stop seeing.
 */
function Waiting({ initiative, labels }: { initiative: Initiative; labels: ExplorerLabels }) {
  const { notes, amendments } = waiting(initiative.rollup)
  if (notes === 0 && amendments === 0) return null
  return (
    <span
      className={cn(
        'flex shrink-0 items-center gap-1 font-mono text-[10.5px] leading-none',
        initiative.status === 'open' && 'ml-auto',
      )}
    >
      {amendments > 0 && (
        <span
          title={labels.waitingAmendments(amendments)}
          aria-label={labels.waitingAmendments(amendments)}
          className="bg-primary text-primary-foreground rounded-full px-1.5 py-0.5 font-semibold"
        >
          {amendments}
        </span>
      )}
      {notes > 0 && (
        <span
          title={labels.waitingNotes(notes)}
          aria-label={labels.waitingNotes(notes)}
          className="border-border text-muted-foreground rounded-full border px-1.5 py-0.5"
        >
          {notes}
        </span>
      )}
    </span>
  )
}

/** Indent per level, capped so a deep path does not push the title off the row. */
function indent(depth: number): number {
  return 8 + Math.min(depth, 6) * 12
}

interface Row {
  node: TreeNode
  depth: number
}

/** Depth-first rows, skipping everything under a collapsed folder. */
function flatten(root: TreeFolder, expanded: ReadonlySet<string>): Row[] {
  const rows: Row[] = []
  const walk = (folder: TreeFolder, depth: number): void => {
    for (const child of folder.children) {
      rows.push({ node: child, depth })
      if (child.kind === 'folder' && expanded.has(child.path)) walk(child, depth + 1)
    }
  }
  walk(root, 0)
  return rows
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      viewBox="0 0 12 12"
      aria-hidden="true"
      className={cn('size-3 shrink-0 transition-transform', open && 'rotate-90')}
    >
      <path d="M4 2.5 L8 6 L4 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  )
}

/** Every ancestor folder of a document, so selecting one can reveal it. */
export function revealPath(initiative: Initiative): string[] {
  return ancestors(pathOf(initiative))
}
