// Editing one thought, in one place.
//
// The canvas card used to be the editor: a title textarea drawn over the node, a
// notes box, two selects, a colour row and a checkbox, all inside a 300×320 box
// that is itself drawn OVER its neighbours. It worked, and it cost more than it
// was worth. A form on the canvas has to fight the canvas for the keyboard —
// Space folds a branch, Enter grows one, Backspace prunes — so the card swallowed
// every keystroke, which then swallowed the keys the canvas needs and pushed ⌘K
// into the capture phase to get out from under it. Reading a map and typing into
// it are two different postures, and only one of them belongs on the map.
//
// So the card is text you read, and this is where it is changed — modelled on
// `AttachmentsDialog`, which had already made the same move for the same reason.
// Everything that was editable inline is here and nothing was dropped on the way:
// title, notes, kind, shape, colour, edge label, the reviewed flag, the relations
// touching this node, and a question's answer.
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
  MAX_TITLE,
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

/**
 * Which field the dialog opens on.
 *
 * Rename and "write notes on it" are the same dialog reached by two verbs, and
 * the only difference between them is where the caret lands — which is the whole
 * reason they can be one dialog rather than two.
 */
export type NodeDialogFocus = 'title' | 'notes'

export interface NodeDialogLabels {
  /** `{title}` is the thought being edited. */
  heading: string
  subtitle: string
  titleField: string
  titleHint: string
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
  /** The thought being edited, or null while closed. */
  node: MapNode | null
  canWrite: boolean
  focus: NodeDialogFocus
  /** Only the relations touching this node. */
  relations: readonly Relationship[]
  /** Titles for the far end of each relation, so a row reads as prose. */
  titleOf: ReadonlyMap<string, string>
  onOpenChange: (open: boolean) => void
  onTitle: (id: string, title: string) => void
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
  focus,
  relations,
  titleOf,
  onOpenChange,
  onTitle,
  onNotes,
  onFields,
  onRemoveRelation,
  onAnswer,
  labels,
}: NodeDialogProps) {
  // Every text field is a draft until it is left. Writing every keystroke into
  // the document would be correct and unkind: it is one shared history, and a
  // paragraph typed slowly would arrive at a collaborator letter by letter.
  const [title, setTitle] = useState(node?.title ?? '')
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
    setTitle(node?.title ?? '')
    setNotes(node?.notes ?? '')
    setEdgeLabel(node?.edge_label ?? '')
    setAnswer('')
  }

  const titleRef = useRef<HTMLInputElement | null>(null)
  const notesRef = useRef<HTMLTextAreaElement | null>(null)
  const id = node?.id ?? null
  // Rename lands in the title with it selected — the common edit is replacing a
  // first-draft thought, not appending to it, and a node created by Enter or Tab
  // arrives here holding a placeholder that is meant to be typed over.
  const focusRef = useRef(focus)
  focusRef.current = focus
  const takeFocus = useCallback(() => {
    if (focusRef.current === 'notes') notesRef.current?.focus()
    else {
      titleRef.current?.focus()
      titleRef.current?.select()
    }
  }, [])
  // On open, from the dialog's own focus event — the effect below runs before
  // Radix has finished settling focus, so it cannot be the only path. The effect
  // is what moves the caret when the SAME open dialog is asked for by the other
  // verb, which fires no open event.
  useEffect(() => {
    if (!id) return
    takeFocus()
  }, [id, focus, takeFocus])

  if (!node) return null

  const isQuestion = node.kind === 'question'
  /** What a question is about, read off the relation that carries it. */
  const aboutId = isQuestion ? questionTarget(relations, node.id) : null
  const about = aboutId ? (titleOf.get(aboutId) ?? aboutId) : null

  const commitTitle = () => {
    if (canWrite && title !== node.title) onTitle(node.id, title)
  }
  const commitNotes = () => {
    if (canWrite && notes !== node.notes) onNotes(node.id, notes)
  }
  const commitEdgeLabel = () => {
    if (canWrite && edgeLabel !== node.edge_label) onFields(node.id, { edge_label: edgeLabel })
  }

  const close = () => {
    // Whatever is still in a field is part of the thought, whether it was left
    // by Tab, by Escape or by the close button.
    commitTitle()
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
        // The caret goes where the verb asked for it, which is not the first
        // focusable thing in the dialog whenever the verb was "write notes".
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

          <label className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">
              {labels.titleField}
            </span>
            <Input
              ref={titleRef}
              aria-label={labels.titleField}
              value={title}
              disabled={!canWrite}
              maxLength={MAX_TITLE}
              placeholder={labels.titleHint}
              onChange={(e) => setTitle(e.target.value)}
              onBlur={commitTitle}
              onKeyDown={(e) => {
                // Enter is done: naming a thought somebody just created should
                // cost a name and one key, the way it did when the editor was on
                // the canvas.
                if (e.key === 'Enter') {
                  e.preventDefault()
                  close()
                }
              }}
              className="h-8 text-[12.5px]"
            />
          </label>

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
                  // The notes and the title are part of the thought too, and
                  // answering ends this question — so they go first or they go
                  // nowhere.
                  commitTitle()
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
