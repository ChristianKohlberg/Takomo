// The same tree, as an indented list. What a phone gets instead of the canvas.
//
// Not a degraded canvas — a better shape for the screen. A pinch-zoom surface on
// a 375 px viewport is a worse way to read a tree than a list is, and the repo's
// mobile lint rules exist precisely to stop a desktop shape being shipped as
// though it worked. The list also happens to be the fastest thing to *add* to
// with a thumb, which is what somebody on a phone is doing with a brainstorm.
//
// It carries the same one caret the canvas does, and no more. Naming a thought is
// what a brainstorm is made of, so a row being named shows a title caret in place
// — `Aa` opens it on a row that already has a name — and everything else about a
// thought is `NodeDialog` behind the `✎`. That is the same split as on the canvas
// rather than a second, worse editor for a small screen.
//
// Those last two are here because the affordances that carry them on the canvas
// are pointer-driven — a badge, a `+` on hover, a right-click menu — and a phone
// has no hover and no right button. Rather than give the list a worse version of
// each, every row carries the same four verbs as plain buttons.
import { useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import {
  NodeNameInput,
  type NameThen,
  type NodeNameInputLabels,
} from '@/components/mindmap/NodeNameInput'
import { cn } from '@/lib/utils'
import { childrenOf } from '@/lib/mindmap-layout'
import type { MapNode } from '@/lib/mindmap-doc'
import { firstSentence, trustOf, type FoldSummary, type Trust } from '@/lib/mindmap-lens'

export interface OutlineLabels {
  /** Opens the reading-and-editing dialog on this row. */
  edit: string
  actions?: string
  /** Opens the title caret in place — the phone's F2. */
  rename: string
  /** The caret itself: its accessible name, and its placeholder. */
  nameField: string
  nameHint: string
  addChild: string
  addSibling: string
  empty: string
  /** Marks a node somebody wrote notes on — the long form lives off the canvas. */
  hasNotes: string
  /** The attachments button. `{n}` is the count, zero included — on a phone this
   *  is the only way in, so it is present even when there is nothing yet. */
  attachments: string
  remove: string
  /** Detaching a row from its parent — the phone's route to cutting an edge,
   *  which on the canvas is a click on a line a thumb cannot aim at. */
  detach: string
  /** A folded branch, summarised. `{n}` is the count. */
  folded: string
  /** The eyebrow on a question node. */
  question: string
  /** The trust lens, in words. A dot alone says nothing to a screen reader. */
  trustConfirmed: string
  trustMachine: string
  trustUnverified: string
}

export interface OutlineProps {
  nodes: MapNode[]
  selected: string | null
  canWrite: boolean
  onSelect: (id: string) => void
  /** Opens the same dialog the pill's open verb opens on the canvas. Present on
   *  a read-only token too: it is where the whole of a thought can be READ, and
   *  it refuses every write by itself. */
  onEdit: (id: string) => void
  /** The row whose title is being typed right now, or null. */
  naming: string | null
  /** Opens the caret on a row that already has a name. */
  onRename: (id: string) => void
  onNameCommit: (id: string, title: string, then: NameThen) => void
  onNameCancel: (id: string) => void
  onChild: (id: string) => void
  onSibling: (id: string) => void
  /** Opens the same manager the canvas badge opens. */
  onAttachments: (id: string) => void
  /** Goes through the same two questions the canvas does. */
  onDelete: (id: string) => void
  /** Also two questions, and also not a deletion. */
  onDetach: (id: string) => void
  /** What each folded branch is holding — the hidden rows are not in `nodes`. */
  foldSummaryOf: (id: string) => FoldSummary | null
  /** The same lens the canvas has, so the phone can ask the same question. */
  trustLens: boolean
  labels: OutlineLabels
  className?: string
}

export function Outline({
  nodes,
  selected,
  canWrite,
  onSelect,
  onEdit,
  naming,
  onRename,
  onNameCommit,
  onNameCancel,
  onChild,
  onSibling,
  onAttachments,
  onDelete,
  onDetach,
  foldSummaryOf,
  trustLens,
  labels,
  className,
}: OutlineProps) {
  const kids = childrenOf(nodes)
  const [menuFor, setMenuFor] = useState<string | null>(null)
  const chose = useRef(false)

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
      {rows.map(({ node, depth }) => {
        const fold = foldSummaryOf(node.id)
        // The same one line of substance the canvas draws, for the same reason:
        // a list you scroll should read as thoughts, not as labels.
        const substance = fold ? fold.text : firstSentence(node.notes, 140)
        const trust: Trust | null =
          trustLens && node.kind !== 'question' ? trustOf(node) : null
        const nameLabels: NodeNameInputLabels = {
          field: labels.nameField,
          hint: labels.nameHint,
        }
        const trustLabel =
          trust === 'confirmed'
            ? labels.trustConfirmed
            : trust === 'machine'
              ? labels.trustMachine
              : labels.trustUnverified
        return (
        <li
          key={node.id}
          className={cn(
            'border-b-border-soft flex flex-wrap items-start gap-2 border-b py-2.5 pr-2',
            selected === node.id && 'bg-accent',
          )}
          // Indentation is inline because it is data, not a style: a depth-4 node
          // needs a depth-4 inset, and Tailwind cannot spell an arbitrary one
          // without generating a class per level.
          style={{ paddingLeft: `${12 + Math.min(depth, 3) * 12}px` }}
        >
          {naming === node.id && canWrite ? (
            // Named in place, for the same reason the canvas is: a modal per new
            // thought is too heavy for what a brainstorm is for.
            <div className="min-w-0 flex-1">
              <NodeNameInput
                value={node.title}
                onCommit={(text, then) => onNameCommit(node.id, text, then)}
                onCancel={() => onNameCancel(node.id)}
                labels={nameLabels}
              />
            </div>
          ) : (
          <button
            type="button"
            onClick={() => onSelect(node.id)}
            className="min-w-0 flex-1 cursor-pointer text-left"
          >
            {node.kind === 'question' && (
              <div className="font-mono text-[9px] font-[650] tracking-wider text-violet-600 uppercase dark:text-violet-300">
                ? {labels.question}
              </div>
            )}
            <div className="text-foreground break-words text-[13px] leading-snug">
              {trust && (
                <span className="mr-1.5 text-[11px]" title={trustLabel}>
                  {trust === 'confirmed' ? '✓' : trust === 'machine' ? '⌁' : '~'}
                </span>
              )}
              {node.title}
              {/* The same marks the canvas draws, for the same reason: a list
                  you can scroll still shows where the substance is. */}
              {node.notes && (
                <span className="text-muted-foreground ml-1.5" title={labels.hasNotes}>
                  ≋
                </span>
              )}
              {fold && (
                <span
                  className="text-muted-foreground ml-1.5 font-mono text-[10.5px]"
                  title={labels.folded.replace('{n}', String(fold.count))}
                >
                  ⊞ {fold.count}
                </span>
              )}
            </div>
            {substance && (
              <div className="text-muted-foreground line-clamp-2 text-[11.5px] leading-snug">
                {substance}
              </div>
            )}
            {node.promoted && (
              <div className="text-muted-foreground truncate font-mono text-[10.5px]">
                → {node.promoted.kind} {node.promoted.id}
              </div>
            )}
          </button>
          )}
          <Popover open={menuFor === node.id} onOpenChange={open => { if (open) chose.current = false; setMenuFor(open ? node.id : null) }}>
            <PopoverTrigger asChild>
              <Button variant="outline" size="icon" className="shrink-0 text-lg" aria-label={labels.actions ?? labels.edit}>⋯</Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-52 gap-0 p-1" onCloseAutoFocus={event => { if (chose.current) event.preventDefault() }}>
              {[
                { label: labels.edit, action: () => onEdit(node.id) },
                { label: labels.attachments.replace('{n}', String(node.attachments.length)), action: () => onAttachments(node.id) },
                ...(canWrite ? [
                  { label: labels.rename, action: () => onRename(node.id) },
                  { label: labels.addChild, action: () => onChild(node.id) },
                  { label: labels.addSibling, action: () => onSibling(node.id) },
                  ...(node.parent !== null ? [{ label: labels.detach, action: () => onDetach(node.id) }] : []),
                  { label: labels.remove, action: () => onDelete(node.id) },
                ] : []),
              ].map(item => <Button key={item.label} variant="ghost" size="sm" className="h-auto min-h-9 justify-start whitespace-normal text-left" onClick={() => {
                chose.current = true
                setMenuFor(null)
                onSelect(node.id)
                item.action()
              }}>{item.label}</Button>)}
            </PopoverContent>
          </Popover>
          {selected === node.id && naming !== node.id && <div className="flex w-full flex-wrap gap-1" role="group" aria-label={labels.actions ?? labels.edit}>
            <Button variant="outline" size="sm" onClick={() => onEdit(node.id)}>{labels.edit}</Button>
            {canWrite && <Button variant="outline" size="sm" onClick={() => onChild(node.id)}>{labels.addChild}</Button>}
          </div>}
        </li>
        )
      })}
    </ul>
  )
}
