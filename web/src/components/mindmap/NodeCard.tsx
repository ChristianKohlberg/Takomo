// One node's card — and, when it is the selected one, everything about that node.
//
// This replaces the side panel. A panel at the edge of the page answers "what is
// this node" a long way from the node, and it is permanently there whether or not
// anybody asked; the detail belongs ON the thing, where the pointer already is.
//
// The rule that keeps that from ruining a 500-node map is that only the SELECTED
// card grows. Every other node stays a title and a row of marks — `≋` where there
// are notes, `¶` where lines run to other branches — so a map you are reading
// still shows you where the substance is without drawing any of it.
//
// THE CARD IS TEXT YOU READ, NEVER TEXT YOU TYPE INTO. There is no input, no
// textarea, no select and no checkbox anywhere in here, and there is no inline
// title editor over the node either. Editing one thought is a dialog
// (`NodeDialog`), for the reason the attachment list already moved into one: a
// form inside a 300×320 box that is drawn OVER its neighbours is a form whose
// every control has to fight the canvas for the keyboard, and it lost that fight
// in both directions — the card had to swallow every keystroke so Space on a
// checkbox would not fold a branch, which then swallowed the keys the canvas
// needs.
//
// What is left is a reading surface: the title, the full notes, what is attached,
// the lines running to other branches, what this thought became, and who wrote
// it. Nothing here stops a pointer any more, so the selected node drags from
// anywhere on its card — only the wheel is caught, so scrolling long notes does
// not zoom the map.
import { firstSentence, type FoldSummary, type Trust } from '@/lib/mindmap-lens'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'
import { cn } from '@/lib/utils'

/** One letter for the node's kind. A full word would not fit and does not need to. */
const KIND_MARK: Record<MapNode['kind'], string> = {
  thought: '',
  question: '?',
  decision: '!',
  screen: '▢',
  component: '◧',
}

/**
 * How big the selected card is drawn, in world units.
 *
 * It overlaps its neighbours rather than pushing them, and that is the trade:
 * re-laying the map out around whatever is selected would move every other node
 * under a collaborator's cursor every time somebody clicked.
 */
export const EXPANDED_WIDTH = 300
export const EXPANDED_HEIGHT = 320

export interface NodeCardLabels {
  notes: string
  /** The attachment list's heading. `{n}` is the count. */
  attachments: string
  relations: string
  noRelations: string
  promoted: string
  origin: string
  originHuman: string
  originAgent: string
  /** Tooltips on the marks a collapsed card shows instead of the detail. */
  hasNotes: string
  hasRelations: string
  /** The eyebrow on a question node. */
  question: string
  /** What a folded branch is holding. `{n}` is the count. */
  folded: string
  /** The trust lens, said in words so the reading is never colour alone. */
  trustConfirmed: string
  trustMachine: string
  trustUnverified: string
}

export interface NodeCardProps {
  node: MapNode
  /** Only the selected card expands; every other one stays a title and marks. */
  expanded: boolean
  /** Only the relations touching this node. */
  relations: readonly Relationship[]
  /** Titles for the far end of each relation, so a row reads as prose. */
  titleOf: ReadonlyMap<string, string>
  /** What this viewer folded away under this node, or null. Folding SUMMARISES:
   *  the card says how much went and names it. */
  fold: FoldSummary | null
  /** How confident we are in this node, or null when the lens is off. */
  trust: Trust | null
  labels: NodeCardLabels
  className?: string
}

export function NodeCard({
  node,
  expanded,
  relations,
  titleOf,
  fold,
  trust,
  labels,
  className,
}: NodeCardProps) {
  const mark = KIND_MARK[node.kind]
  // Attachments are NOT counted here any more: the badge on the node draws them,
  // and a mark that added them to the relation count would say a number nothing
  // on screen agrees with.
  const context = relations.length > 0
  const isQuestion = node.kind === 'question'

  /**
   * The one line of substance an unselected card carries.
   *
   * A folded branch says what is inside it; otherwise the first sentence of the
   * notes. That is what turns a map of thirty nodes into thirty thoughts rather
   * than thirty labels — and the full notes are still one click away, on the
   * card that grows.
   */
  const substance = fold ? fold.text : firstSentence(node.notes, 140)

  /** In words, so the lens is never colour alone. */
  const trustLabel: Record<Trust, string> = {
    confirmed: labels.trustConfirmed,
    machine: labels.trustMachine,
    unverified: labels.trustUnverified,
  }
  const TRUST_MARK: Record<Trust, string> = { confirmed: '✓', machine: '⌁', unverified: '~' }

  const titleBlock = (
    <div
      className={cn(
        'text-foreground text-[12.5px] leading-snug',
        !expanded && (substance || isQuestion ? 'line-clamp-1' : 'line-clamp-2'),
      )}
    >
      {mark ? `${mark} ` : ''}
      {node.title}
    </div>
  )

  const eyebrow = isQuestion && (
    <div className="font-mono text-[9px] leading-tight font-[650] tracking-wider text-violet-600 uppercase dark:text-violet-300">
      ? {labels.question}
    </div>
  )

  if (!expanded) {
    return (
      <div
        className={cn(
          'flex h-full flex-col justify-center overflow-hidden px-2.5',
          substance ? 'py-1' : 'gap-0.5 py-1.5',
          className,
        )}
      >
        {/* A question is not a thought and does not read like one. */}
        {eyebrow}
        {titleBlock}
        {/* The marks. A collapsed map still says WHERE the substance is, which is
            the whole reason it is safe to draw only one card in full. Absent
            rather than empty: with an eyebrow and a line of substance above and
            below it, a blank row is a line of card height spent on nothing. */}
        {(node.promoted || node.notes || context || node.origin === 'agent' || trust || fold) && (
        <div className="text-muted-foreground flex items-center gap-1.5 truncate font-mono text-[10px]">
          {node.promoted && <span>→ {node.promoted.kind}</span>}
          {node.notes && <span title={labels.hasNotes}>≋</span>}
          {context && (
            <span title={labels.hasRelations}>¶ {relations.length}</span>
          )}
          {node.origin === 'agent' && <span>⌁</span>}
          {trust && <span title={trustLabel[trust]}>{TRUST_MARK[trust]}</span>}
          {fold && (
            <span className="ml-auto shrink-0" title={labels.folded.replace('{n}', String(fold.count))}>
              ⊞ {fold.count}
            </span>
          )}
        </div>
        )}
        {/* One line of what this node actually SAYS — the titles a fold is
            standing in for, or the first sentence of its notes. */}
        {substance && (
          <div className="text-muted-foreground line-clamp-1 text-[10.5px] leading-tight">
            {substance}
          </div>
        )}
      </div>
    )
  }

  return (
    <div
      className={cn('flex h-full min-h-0 flex-col', className)}
      // The one event the card still catches. Scrolling long notes must not zoom
      // the map behind them; everything else — press, double-click, keystroke —
      // belongs to the canvas, which is what keeps the selected node draggable
      // from anywhere on it and leaves ⌘K, the arrows and Delete alone.
      onWheel={(e) => e.stopPropagation()}
    >
      <div className="border-b-border-soft text-muted-foreground flex items-center gap-1.5 border-b px-2 py-1 font-mono text-[10px]">
        <span className="min-w-0 truncate">
          {labels.origin} {node.origin === 'agent' ? labels.originAgent : labels.originHuman}
        </span>
        {trust && (
          <span className="shrink-0" title={trustLabel[trust]}>
            {TRUST_MARK[trust]}
          </span>
        )}
        {node.promoted && (
          <span className="text-foreground ml-auto shrink-0">
            → {labels.promoted} {node.promoted.id}
          </span>
        )}
      </div>

      <div className="flex min-h-0 flex-col gap-2 overflow-y-auto px-2 py-2">
        {eyebrow}
        {titleBlock}

        {node.notes && (
          <div className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.notes}</span>
            <p className="text-foreground m-0 text-[11.5px] leading-snug whitespace-pre-wrap">
              {node.notes}
            </p>
          </div>
        )}

        {/* What is attached, by name. The badge on the node carries the count and
            the dialog behind it is where one is added or corrected; this is the
            reading of it, which the card could not carry while it was a form. */}
        {node.attachments.length > 0 && (
          <div className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">
              {labels.attachments.replace('{n}', String(node.attachments.length))}
            </span>
            <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
              {node.attachments.map((a) => (
                <li key={a.id} className="text-foreground truncate text-[11px]">
                  ⎘ {a.name}
                  {a.gist ? ` · ${a.gist}` : ''}
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* ---- relations ---- */}
        <div className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.relations}</span>
          {relations.length === 0 ? (
            <span className="text-muted-foreground text-[10.5px]">{labels.noRelations}</span>
          ) : (
            <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
              {relations.map((r) => {
                const other = r.from === node.id ? r.to : r.from
                return (
                  <li key={r.id} className="truncate text-[11px]">
                    {r.from === node.id ? '→' : '←'} {titleOf.get(other) ?? other}
                    {r.label ? ` · ${r.label}` : ''}
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      </div>
    </div>
  )
}
