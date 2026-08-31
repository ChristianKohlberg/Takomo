// Reading and changing one thought, in one place.
//
// Two moves made this the only detail surface there is. The canvas card used to
// be the editor — a title textarea drawn over the node, a notes box, two selects,
// a colour row and a checkbox, all inside a box drawn OVER its neighbours — and a
// form on the canvas has to fight the canvas for the keyboard, so the card
// swallowed every keystroke and then swallowed the keys the canvas needs. And
// SELECTING a node used to expand that card into a 300×320 reading panel, so
// every click on the map threw a panel across it whether or not anybody had asked
// to open anything. Selecting is not opening.
//
// So the card is a title and its marks, and this is where a thought is read and
// changed. Everything the expanded card showed is here — who wrote it, what it
// became, the whole notes, what is attached, the lines running to other branches
// — alongside everything that was ever editable inline.
//
// THE ONE FIELD THAT IS NOT HERE IS THE TITLE. It is the heading instead, read
// only: a title is typed on the node itself, with the inline caret that opens on
// creation, on rename, on F2 and on double-click. Two ways to edit one field is
// the trap this repo names elsewhere, so there is exactly one.
//
// The explicit-fields decision is what makes this safe to hand to two people at
// once: `color`, `shape`, `kind` and `edge_label` are separate keys in the
// document, so recolouring and re-labelling the same node at the same time both
// land. A single `style` blob would have merged as a whole and one of them would
// have silently lost.
//
// There is no save button, for the reason `/documents` has none: this is a shared
// document, so the honest question is not "did my change save" but "am I
// connected". A field commits when you leave it and when the dialog closes —
// including on Escape, which cannot be a discard here without inventing a dirty
// state the rest of the surface does not have.
import { useCallback, useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  MAX_NOTES,
  NODE_KINDS,
  type MapNode,
  type NodeFields,
  type NodeKind,
  type Relationship,
} from '@/lib/mindmap-doc'
import { questionTarget } from '@/lib/mindmap-lens'
import { cn } from '@/lib/utils'

/** Node shapes. Explicit, and stored as a plain string like every other field. */
export const NODE_SHAPES = ['rounded', 'square', 'pill'] as const
export type NodeShape = (typeof NODE_SHAPES)[number]

/** A small fixed palette. A colour picker would invite 16 million answers to a
 *  question whose useful answers number about six. */
export const NODE_COLORS = ['', '#fee2e2', '#ffedd5', '#fef9c3', '#dcfce7', '#dbeafe', '#f3e8ff']

export interface NodeDialogLabels {
  /** `{title}` is the thought being read. The title itself is not a field here. */
  heading: string
  subtitle: string
  /** Who wrote the thought — what the expanded card carried in its top strip. */
  origin: string
  originHuman: string
  originAgent: string
  /** What this branch became, when it graduated. */
  promoted: string
  /** The attachment list's heading. `{n}` is the count. */
  attachments: string
  noAttachments: string
  /** Through to the manager, which is where one is added or corrected. */
  openAttachments: string
  notes: string
  notesHint: string
  notesCount: string
  kind: string
  shape: string
  color: string
  colorNone: string
  edgeLabel: string
  edgeLabelHint: string
  reviewed: string
  relations: string
  removeRelation: string
  noRelations: string
  close: string
  readOnly: string
  kinds: Record<NodeKind, string>
  shapes: Record<NodeShape, string>
  /** The eyebrow on a question node, and what its answer box is called. */
  question: string
  answer: string
  answerHint: string
  answerAction: string
  /** `{title}` is the thought the question is about. */
  answerAbout: string
  answerAlone: string
}

export interface NodeDialogProps {
  /** The thought being read, or null while closed. */
  node: MapNode | null
  canWrite: boolean
  /** Only the relations touching this node. */
  relations: readonly Relationship[]
  /** Titles for the far end of each relation, so a row reads as prose. */
  titleOf: ReadonlyMap<string, string>
  onOpenChange: (open: boolean) => void
  /** Hands over to `AttachmentsDialog`. Attachments are READ here and changed
   *  there, which is the same one-surface-per-field rule the title follows. */
  onOpenAttachments: (id: string) => void
  onNotes: (id: string, notes: string) => void
  onFields: (id: string, fields: Partial<NodeFields>) => void
  onRemoveRelation: (relationId: string) => void
  /** A person's own answer to a question node. There is no model in this path. */
  onAnswer: (id: string, answer: string) => void
  labels: NodeDialogLabels
}

const FIELD =
  'border-border-soft bg-card text-foreground w-full rounded-md border px-2 py-1.5 text-[12px] disabled:opacity-50'

export function NodeDialog({
  node,
  canWrite,
  relations,
  titleOf,
  onOpenChange,
  onOpenAttachments,
  onNotes,
  onFields,
  onRemoveRelation,
  onAnswer,
  labels,
}: NodeDialogProps) {
  // Every text field is a draft until it is left. Writing every keystroke into
  // the document would be correct and unkind: it is one shared history, and a
  // paragraph typed slowly would arrive at a collaborator letter by letter.
  const [notes, setNotes] = useState(node?.notes ?? '')
  const [edgeLabel, setEdgeLabel] = useState(node?.edge_label ?? '')
  // A question's answer is never written into the document as it is typed: it is
  // not the question's own text.
  const [answer, setAnswer] = useState('')

  // The drafts belong to a NODE, not to the dialog, so a different node re-seeds
  // them. Done by comparing the id during render — React's own
  // adjust-state-on-prop-change pattern — rather than in an effect: an effect
  // keyed on the id alone would be a lie to the dependency linter, and one keyed
  // honestly on the text would re-seed on every remote keystroke and fight
  // whoever is typing here.
  const [seeded, setSeeded] = useState<string | null>(node?.id ?? null)
  if (seeded !== (node?.id ?? null)) {
    setSeeded(node?.id ?? null)
    setNotes(node?.notes ?? '')
    setEdgeLabel(node?.edge_label ?? '')
    setAnswer('')
  }

  const notesRef = useRef<HTMLTextAreaElement | null>(null)
  const id = node?.id ?? null
  // The notes are what somebody came here to write: the title is typed on the
  // node itself, and everything else on this surface is a control rather than a
  // caret.
  const takeFocus = useCallback(() => notesRef.current?.focus(), [])
  // On open, from the dialog's own focus event — the effect below runs before
  // Radix has finished settling focus, so it cannot be the only path.
  useEffect(() => {
    if (!id) return
    takeFocus()
  }, [id, takeFocus])

  if (!node) return null

  const isQuestion = node.kind === 'question'
  /** What a question is about, read off the relation that carries it. */
  const aboutId = isQuestion ? questionTarget(relations, node.id) : null
  const about = aboutId ? (titleOf.get(aboutId) ?? aboutId) : null

  const commitNotes = () => {
    if (canWrite && notes !== node.notes) onNotes(node.id, notes)
  }
  const commitEdgeLabel = () => {
    if (canWrite && edgeLabel !== node.edge_label) onFields(node.id, { edge_label: edgeLabel })
  }

  const close = () => {
    // Whatever is still in a field is part of the thought, whether it was left
    // by Tab, by Escape or by the close button.
    commitNotes()
    commitEdgeLabel()
    onOpenChange(false)
  }

  const shape: NodeShape = NODE_SHAPES.includes(node.shape as NodeShape)
    ? (node.shape as NodeShape)
    : 'rounded'

  return (
    <Dialog open onOpenChange={(open) => (open ? onOpenChange(true) : close())}>
      <DialogContent
        className="max-w-[calc(100vw-2rem)] sm:max-w-lg"
        // One close, in the footer: the corner cross would be a second one with
        // the same name, and the footer button is the reachable one on a phone.
        showCloseButton={false}
        // The caret goes to the notes rather than to the first focusable thing,
        // which would be a select nobody opened this to change.
        onOpenAutoFocus={(e) => {
          e.preventDefault()
          takeFocus()
        }}
      >
        <DialogHeader>
          <DialogTitle>{labels.heading.replace('{title}', node.title)}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex max-h-[55vh] flex-col gap-2.5 overflow-y-auto">
          {isQuestion && (
            <div className="font-mono text-[9px] font-[650] tracking-wider text-violet-600 uppercase dark:text-violet-300">
              ? {labels.question}
            </div>
          )}

          {/* Who wrote it, and what it became. The expanded card carried both in
              its top strip; with that card gone this is where they are read. */}
          <div className="text-muted-foreground flex flex-wrap items-center gap-2 font-mono text-[10.5px]">
            <span>
              {labels.origin} {node.origin === 'agent' ? labels.originAgent : labels.originHuman}
            </span>
            {node.promoted && (
              <span className="text-foreground">
                → {labels.promoted} {node.promoted.kind} {node.promoted.id}
              </span>
            )}
          </div>

          <label className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.notes}</span>
            <Textarea
              ref={notesRef}
              aria-label={labels.notes}
              value={notes}
              disabled={!canWrite}
              maxLength={MAX_NOTES}
              rows={5}
              placeholder={labels.notesHint}
              onChange={(e) => setNotes(e.target.value)}
              onBlur={commitNotes}
              className="text-[12px]"
            />
            <span className="text-muted-foreground text-[10px]">
              {labels.notesCount
                .replace('{n}', String(notes.length))
                .replace('{max}', String(MAX_NOTES))}
            </span>
          </label>

          <div className="grid grid-cols-2 gap-1.5">
            <label className="flex flex-col gap-0.5">
              <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.kind}</span>
              <select
                className={FIELD}
                aria-label={labels.kind}
                value={node.kind}
                disabled={!canWrite}
                onChange={(e) => onFields(node.id, { kind: e.target.value as NodeKind })}
              >
                {NODE_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {labels.kinds[k]}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex flex-col gap-0.5">
              <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.shape}</span>
              <select
                className={FIELD}
                aria-label={labels.shape}
                value={shape}
                disabled={!canWrite}
                onChange={(e) => onFields(node.id, { shape: e.target.value })}
              >
                {NODE_SHAPES.map((s) => (
                  <option key={s} value={s}>
                    {labels.shapes[s]}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <label className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">
              {labels.edgeLabel}
            </span>
            <Input
              aria-label={labels.edgeLabel}
              value={edgeLabel}
              disabled={!canWrite}
              placeholder={labels.edgeLabelHint}
              onChange={(e) => setEdgeLabel(e.target.value)}
              onBlur={commitEdgeLabel}
              className="h-8 text-[12px]"
            />
          </label>

          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.color}</span>
            {NODE_COLORS.map((c) => (
              <button
                key={c || 'none'}
                type="button"
                disabled={!canWrite}
                aria-label={c || labels.colorNone}
                aria-pressed={node.color === c}
                onClick={() => onFields(node.id, { color: c })}
                style={c ? { backgroundColor: c } : undefined}
                className={cn(
                  'border-border h-5 w-5 cursor-pointer rounded border text-[10px] disabled:opacity-40',
                  node.color === c && 'ring-ring ring-2',
                )}
              >
                {c ? '' : '×'}
              </button>
            ))}
          </div>

          <label className="flex items-center gap-1.5 text-[12px]">
            <input
              type="checkbox"
              checked={node.reviewed}
              disabled={!canWrite}
              onChange={(e) => onFields(node.id, { reviewed: e.target.checked })}
            />
            <span>{labels.reviewed}</span>
          </label>

          {/* A question is answered in a person's own words, and the answer goes
              to the thought it was ABOUT rather than staying here — which is why
              answering also removes the question. No model is asked anything. */}
          {isQuestion && (
            <div className="flex flex-col gap-1 border-l-2 border-violet-400 pl-2 dark:border-violet-500">
              <span className="text-[10.5px] font-[650] text-violet-700 dark:text-violet-300">
                {labels.answer}
              </span>
              <span className="text-muted-foreground text-[10.5px]">
                {about ? labels.answerAbout.replace('{title}', about) : labels.answerAlone}
              </span>
              <Textarea
                aria-label={labels.answer}
                value={answer}
                disabled={!canWrite}
                rows={2}
                placeholder={labels.answerHint}
                onChange={(e) => setAnswer(e.target.value)}
                className="text-[12px]"
              />
              <Button
                size="sm"
                variant="outline"
                className="w-fit"
                disabled={!canWrite || !answer.trim()}
                onClick={() => {
                  // The notes are part of the thought too, and answering ends
                  // this question — so they go first or they go nowhere.
                  commitNotes()
                  commitEdgeLabel()
                  onAnswer(node.id, answer)
                  setAnswer('')
                }}
              >
                {labels.answerAction}
              </Button>
            </div>
          )}

          {/* ---- attachments ---- */}
          {/* Read here, changed next door. The count badge on the node is the
              other way into the same manager; this is so that nothing the
              expanded card used to show became unreachable when it went. */}
          <div className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[10.5px] font-[650]">
              {labels.attachments.replace('{n}', String(node.attachments.length))}
            </span>
            {node.attachments.length === 0 ? (
              <span className="text-muted-foreground text-[10.5px]">{labels.noAttachments}</span>
            ) : (
              <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
                {node.attachments.map((a) => (
                  <li key={a.id} className="text-foreground truncate text-[12px]">
                    ⎘ {a.name}
                    {a.gist ? ` · ${a.gist}` : ''}
                  </li>
                ))}
              </ul>
            )}
            <Button
              size="sm"
              variant="outline"
              className="w-fit"
              onClick={() => onOpenAttachments(node.id)}
            >
              {labels.openAttachments}
            </Button>
          </div>

          {/* ---- relations ---- */}
          <div className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[10.5px] font-[650]">
              {labels.relations}
            </span>
            {relations.length === 0 ? (
              <span className="text-muted-foreground text-[10.5px]">{labels.noRelations}</span>
            ) : (
              <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
                {relations.map((r) => {
                  const other = r.from === node.id ? r.to : r.from
                  const far = titleOf.get(other) ?? other
                  return (
                    <li key={r.id} className="flex items-center gap-1 text-[12px]">
                      <span className="min-w-0 grow truncate">
                        {r.from === node.id ? '→' : '←'} {far}
                        {r.label ? ` · ${r.label}` : ''}
                      </span>
                      {canWrite && (
                        <button
                          type="button"
                          aria-label={`${labels.removeRelation} — ${far}`}
                          onClick={() => onRemoveRelation(r.id)}
                          className="cursor-pointer px-1"
                        >
                          ×
                        </button>
                      )}
                    </li>
                  )
                })}
              </ul>
            )}
          </div>

          {!canWrite && (
            <p className="text-muted-foreground m-0 text-[11px]">{labels.readOnly}</p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={close}>
            {labels.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
