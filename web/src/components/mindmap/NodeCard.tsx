// One node on the map: a title, its marks, and one line of what it says.
//
// THE CARD DOES NOT GROW. It used to: selecting a node expanded it into a
// 300×320 reading panel drawn over its neighbours, so every click on the map
// threw a panel across it whether or not the reader had asked to open anything.
// Selecting is not opening. Selection now highlights the node, brings up the
// pill and the `+`, and changes nothing else; a thought is READ in `NodeDialog`,
// which already existed, is already the editing surface, and already has a
// read-only state. One surface for "look at this properly" rather than a canvas
// panel and a dialog that overlap.
//
// What is left here is the always-on signal that there is substance to open:
// `≋` where there are notes, `¶ n` where lines run to other branches, `→` what
// the branch became, `⌁` where an agent wrote it, the trust mark when the lens
// is on, and `⊞ n` for a folded branch — plus one quiet line of the notes' first
// sentence, or the titles a fold is standing in for. (Attachments keep their own
// count badge, drawn on the node by `Canvas`, because it is also the way IN to
// them.)
//
// THE ONE THING YOU TYPE HERE IS A TITLE. A node being named renders its title
// as a caret and nothing else — no notes, no marks, no panel — so the thing you
// are naming is the thing you are looking at. `NodeNameInput` owns that caret
// and swallows exactly the keystrokes going into it. Everything else about a
// thought is the dialog; there is no second way to change any field.
import { firstSentence, type FoldSummary, type Trust } from '@/lib/mindmap-lens'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'
import { cn } from '@/lib/utils'
import {
  NodeNameInput,
  type NameThen,
  type NodeNameInputLabels,
} from '@/components/mindmap/NodeNameInput'

/** One letter for the node's kind. A full word would not fit and does not need to. */
const KIND_MARK: Record<MapNode['kind'], string> = {
  thought: '',
  question: '?',
  decision: '!',
  screen: '▢',
  component: '◧',
}

export interface NodeCardLabels {
  promoted: string
  /** Tooltips on the marks that say where the substance is. */
  hasNotes: string
  hasRelations: string
  /** Said on the `⌁` an agent-written thought carries. */
  originAgent: string
  /** The eyebrow on a question node. */
  question: string
  /** What a folded branch is holding. `{n}` is the count. */
  folded: string
  /** The trust lens, said in words so the reading is never colour alone. */
  trustConfirmed: string
  trustMachine: string
  trustUnverified: string
  /** The tests filed against this section. `{n}` is how many. */
  tests: string
  /** `{n}` tests, `{m}` of them not passing. */
  testsFailing: string
}

/** Non-null exactly while this node is the one being named. */
export interface NodeNaming {
  onCommit: (title: string, then: NameThen) => void
  onCancel: () => void
  labels: NodeNameInputLabels
}

export interface NodeCardProps {
  node: MapNode
  /** Only the relations touching this node — the `¶` mark counts them. */
  relations: readonly Relationship[]
  /** What this viewer folded away under this node, or null. Folding SUMMARISES:
   *  the card says how much went and names it. */
  fold: FoldSummary | null
  /** How confident we are in this node, or null when the lens is off. */
  trust: Trust | null
  /**
   * The tests filed against this section, or null where there are none.
   *
   * A COUNT and not a verdict: the map says where the verification is and where
   * it is failing, and `/verification` is where you read what any of it says.
   * Without this the tests screen is a third view of the plan that the plan
   * itself never mentions.
   */
  tests?: { total: number; failing: number } | null
  /** The title caret, when this is the node being named. Null on every other
   *  node, and null on a read-only token, which never gets a caret at all. */
  naming?: NodeNaming | null
  labels: NodeCardLabels
  className?: string
}

export function NodeCard({
  node,
  relations,
  fold,
  trust,
  tests = null,
  naming = null,
  labels,
  className,
}: NodeCardProps) {
  const mark = KIND_MARK[node.kind]
  // Attachments are NOT counted here: the badge on the node draws them, and a
  // mark that added them to the relation count would say a number nothing on
  // screen agrees with.
  const context = relations.length > 0
  const isQuestion = node.kind === 'question'

  /**
   * The one line of substance a card carries.
   *
   * A folded branch says what is inside it; otherwise the first sentence of the
   * notes. That is what turns a map of thirty nodes into thirty thoughts rather
   * than thirty labels — and the whole of it is one command away, in the dialog.
   */
  const substance = fold ? fold.text : firstSentence(node.notes, 140)

  /** In words, so the lens is never colour alone. */
  const trustLabel: Record<Trust, string> = {
    confirmed: labels.trustConfirmed,
    machine: labels.trustMachine,
    unverified: labels.trustUnverified,
  }
  const TRUST_MARK: Record<Trust, string> = { confirmed: '✓', machine: '⌁', unverified: '~' }

  const eyebrow = isQuestion && (
    <div className="font-mono text-[9px] leading-tight font-[650] tracking-wider text-violet-600 uppercase dark:text-violet-300">
      ? {labels.question}
    </div>
  )

  // Being named: the title line and nothing else, so what is on screen is what
  // is being typed.
  if (naming) {
    return (
      <div className={cn('flex h-full flex-col justify-center gap-0.5 px-2 py-1.5', className)}>
        {eyebrow}
        <NodeNameInput
          value={node.title}
          onCommit={naming.onCommit}
          onCancel={naming.onCancel}
          labels={naming.labels}
        />
      </div>
    )
  }

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
      <div
        className={cn(
          'text-foreground text-[12.5px] leading-snug',
          substance || isQuestion ? 'line-clamp-1' : 'line-clamp-2',
        )}
      >
        {mark ? `${mark} ` : ''}
        {node.title}
      </div>
      {/* The marks. The map says WHERE the substance is without drawing any of
          it — which is what makes opening a thought a separate act rather than
          something a click does to you. Absent rather than empty: with an
          eyebrow and a line of substance above and below it, a blank row is a
          line of card height spent on nothing. */}
      {(node.promoted ||
        node.notes ||
        context ||
        node.origin === 'agent' ||
        trust ||
        tests ||
        fold) && (
        <div className="text-muted-foreground flex items-center gap-1.5 truncate font-mono text-[10px]">
          {node.promoted && (
            <span title={`${labels.promoted} ${node.promoted.id}`}>→ {node.promoted.kind}</span>
          )}
          {node.notes && <span title={labels.hasNotes}>≋</span>}
          {context && <span title={labels.hasRelations}>¶ {relations.length}</span>}
          {node.origin === 'agent' && <span title={labels.originAgent}>⌁</span>}
          {trust && <span title={trustLabel[trust]}>{TRUST_MARK[trust]}</span>}
          {/* Failing is louder than covered: a section with tests that do not
              pass is the one thing on this map somebody has to act on. */}
          {tests && (
            <span
              className={cn(tests.failing > 0 && 'text-nf font-[650]')}
              title={
                tests.failing > 0
                  ? labels.testsFailing
                      .replace('{n}', String(tests.total))
                      .replace('{m}', String(tests.failing))
                  : labels.tests.replace('{n}', String(tests.total))
              }
            >
              ⛉ {tests.total}
              {tests.failing > 0 ? ` · ${tests.failing}` : ''}
            </span>
          )}
          {fold && (
            <span
              className="ml-auto shrink-0"
              title={labels.folded.replace('{n}', String(fold.count))}
            >
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
