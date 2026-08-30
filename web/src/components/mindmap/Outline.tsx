// The same tree, as an indented list. What a phone gets instead of the canvas.
//
// Not a degraded canvas — a better shape for the screen. A pinch-zoom surface on
// a 375 px viewport is a worse way to read a tree than a list is, and the repo's
// mobile lint rules exist precisely to stop a desktop shape being shipped as
// though it worked. The list also happens to be the fastest thing to *add* to
// with a thumb, which is what somebody on a phone is doing with a brainstorm.
//
// It is deliberately not an editor: on a phone you read, tap to select, add,
// remove, and open what is attached. Retyping and rearranging are desktop work.
//
// Those last two are here because the affordances that carry them on the canvas
// are pointer-driven — a badge, a `+` on hover, a right-click menu — and a phone
// has no hover and no right button. Rather than give the list a worse version of
// each, every row carries the same four verbs as plain buttons.
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { childrenOf } from '@/lib/mindmap-layout'
import type { MapNode } from '@/lib/mindmap-doc'
import { Hint } from '@/components/Hint'

export interface OutlineLabels {
  addChild: string
  addSibling: string
  empty: string
  /** Marks a node somebody wrote notes on — the long form lives off the canvas. */
  hasNotes: string
  /** The attachments button. `{n}` is the count, zero included — on a phone this
   *  is the only way in, so it is present even when there is nothing yet. */
  attachments: string
  remove: string
}

export interface OutlineProps {
  nodes: MapNode[]
  selected: string | null
  canWrite: boolean
  onSelect: (id: string) => void
  onChild: (id: string) => void
  onSibling: (id: string) => void
  /** Opens the same manager the canvas badge opens. */
  onAttachments: (id: string) => void
  /** Goes through the same two questions the canvas does. */
  onDelete: (id: string) => void
  labels: OutlineLabels
  className?: string
}

export function Outline({
  nodes,
  selected,
  canWrite,
  onSelect,
  onChild,
  onSibling,
  onAttachments,
  onDelete,
  labels,
  className,
}: OutlineProps) {
  const kids = childrenOf(nodes)

  const rows: { node: MapNode; depth: number }[] = []
  const walk = (parent: string | null, depth: number) => {
    for (const node of kids.get(parent) ?? []) {
      rows.push({ node, depth })
      walk(node.id, depth + 1)
    }
  }
  walk(null, 0)

  if (rows.length === 0) {
    return (
      <div className={cn('text-muted-foreground px-5 py-10 text-center text-[13px]', className)}>
        {labels.empty}
      </div>
    )
  }

  return (
    <ul className={cn('flex flex-col', className)}>
      {rows.map(({ node, depth }) => (
        <li
          key={node.id}
          className={cn(
            'border-b-border-soft flex items-center gap-2 border-b py-2.5 pr-2',
            selected === node.id && 'bg-accent',
          )}
          // Indentation is inline because it is data, not a style: a depth-4 node
          // needs a depth-4 inset, and Tailwind cannot spell an arbitrary one
          // without generating a class per level.
          style={{ paddingLeft: `${12 + depth * 16}px` }}
        >
          <button
            type="button"
            onClick={() => onSelect(node.id)}
            className="min-w-0 grow cursor-pointer text-left"
          >
            <div className="text-foreground text-[13px] leading-snug">
              {node.title}
              {/* The same marks the canvas draws, for the same reason: a list
                  you can scroll still shows where the substance is. */}
              {node.notes && (
                <span className="text-muted-foreground ml-1.5" title={labels.hasNotes}>
                  ≋
                </span>
              )}
            </div>
            {node.promoted && (
              <div className="text-muted-foreground truncate font-mono text-[10.5px]">
                → {node.promoted.kind} {node.promoted.id}
              </div>
            )}
          </button>
          <div className="flex shrink-0 items-center">
            <Hint text={labels.attachments.replace('{n}', String(node.attachments.length))}>
              <Button
                variant="ghost"
                size="sm"
                aria-label={labels.attachments.replace('{n}', String(node.attachments.length))}
                onClick={() => onAttachments(node.id)}
              >
                ⎘{node.attachments.length > 0 ? ` ${node.attachments.length}` : ''}
              </Button>
            </Hint>
            {canWrite && (
              <>
                <Hint text={labels.addSibling}>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={labels.addSibling}
                    onClick={() => onSibling(node.id)}
                  >
                    +
                  </Button>
                </Hint>
                <Hint text={labels.addChild}>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={labels.addChild}
                    onClick={() => onChild(node.id)}
                  >
                    ⇥
                  </Button>
                </Hint>
                <Hint text={labels.remove}>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={labels.remove}
                    onClick={() => onDelete(node.id)}
                  >
                    ×
                  </Button>
                </Hint>
              </>
            )}
          </div>
        </li>
      ))}
    </ul>
  )
}
