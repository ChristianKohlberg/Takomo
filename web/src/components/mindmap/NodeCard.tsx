// One node's card — and, when it is the selected one, everything about that node.
//
// This replaces the side panel. A panel at the edge of the page answers "what is
// this node" a long way from the node, and it is permanently there whether or not
// anybody asked; the detail belongs ON the thing, where the pointer already is.
//
// The rule that keeps that from ruining a 500-node map is that only the SELECTED
// card grows. Every other node stays a title and a row of marks — `≋` where there
// are notes, `¶` where there is context hanging off it — so a map you are reading
// still shows you where the substance is without drawing any of it.
//
// The explicit-fields decision is what makes the editors here safe to hand to two
// people at once: `color`, `shape`, `kind` and `edge_label` are separate keys in
// the document, so recolouring and re-labelling the same node at the same time
// both land. A single `style` blob would have merged as a whole and one of them
// would have silently lost.
import { useEffect, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  ATTACHMENT_KINDS,
  MAX_ATTACHMENTS,
  MAX_NOTES,
  NODE_KINDS,
  type AttachmentKind,
  type MapNode,
  type NodeFields,
  type Relationship,
} from '@/lib/mindmap-doc'
import { cn } from '@/lib/utils'

/**
 * How big the selected card is drawn, in world units.
 *
 * It overlaps its neighbours rather than pushing them, and that is the trade:
 * re-laying the map out around whatever is selected would move every other node
 * under a collaborator's cursor every time somebody clicked.
 */
export const EXPANDED_WIDTH = 300
export const EXPANDED_HEIGHT = 320

/** Node shapes. Explicit, and stored as a plain string like every other field. */
export const NODE_SHAPES = ['rounded', 'square', 'pill'] as const
export type NodeShape = (typeof NODE_SHAPES)[number]

/** A small fixed palette. A colour picker would invite 16 million answers to a
 *  question whose useful answers number about six. */
export const NODE_COLORS = ['', '#fee2e2', '#ffedd5', '#fef9c3', '#dcfce7', '#dbeafe', '#f3e8ff']

/** What ⌘K asked the card to open when it selected this node. */
export type Reveal = 'notes' | 'attach' | null

export interface NodeCardLabels {
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
  origin: string
  originHuman: string
  originAgent: string
  attachments: string
  attachmentsFull: string
  attachmentKind: string
  attachmentName: string
  attachmentGist: string
  attachmentRef: string
  addAttachment: string
  removeAttachment: string
  relations: string
  removeRelation: string
  noRelations: string
  promoted: string
  readOnly: string
  /** Tooltips on the marks a collapsed card shows instead of the detail. */
  hasNotes: string
  hasContext: string
  kindThought: string
  kindQuestion: string
  kindDecision: string
  kindScreen: string
  kindComponent: string
  shapeRounded: string
  shapeSquare: string
  shapePill: string
  attPdf: string
  attCode: string
  attTable: string
  attDiagram: string
  attAudio: string
  attLink: string
}

export interface NodeCardProps {
  node: MapNode
  /** Only the selected card expands; every other one stays a title and marks. */
  expanded: boolean
  canWrite: boolean
  /** The title editor, owned by the canvas so double-click and F2 still open it. */
  editing: boolean
  draft: string
  onDraft: (value: string) => void
  onCommit: () => void
  onCancel: () => void
  onEdit: () => void
  /** Only the relations touching this node. */
  relations: readonly Relationship[]
  /** Titles for the far end of each relation, so a row reads as prose. */
  titleOf: ReadonlyMap<string, string>
  onNotes: (id: string, notes: string) => void
  onFields: (id: string, fields: Partial<NodeFields>) => void
  onAddAttachment: (
    id: string,
    draft: { kind: AttachmentKind; name: string; gist: string; ref: string },
  ) => void
  onRemoveAttachment: (id: string, attachmentId: string) => void
  onRemoveRelation: (relationId: string) => void
  reveal: Reveal
  onRevealed: () => void
  labels: NodeCardLabels
  className?: string
}

/** One letter for the node's kind. A full word would not fit and does not need to. */
const KIND_MARK: Record<MapNode['kind'], string> = {
  thought: '',
  question: '?',
  decision: '!',
  screen: '▢',
  component: '◧',
}

const FIELD =
  'border-border-soft bg-card text-foreground w-full rounded-md border px-1.5 py-1 text-[11.5px]'

export function NodeCard({
  node,
  expanded,
  canWrite,
  editing,
  draft,
  onDraft,
  onCommit,
  onCancel,
  onEdit,
  relations,
  titleOf,
  onNotes,
  onFields,
  onAddAttachment,
  onRemoveAttachment,
  onRemoveRelation,
  reveal,
  onRevealed,
  labels,
  className,
}: NodeCardProps) {
  // The notes box is a draft until it blurs. Writing every keystroke into the
  // document would be correct and unkind: it is one shared history, and a
  // paragraph typed slowly would arrive at a collaborator letter by letter.
  const [notes, setNotes] = useState(node.notes)
  const [edgeLabel, setEdgeLabel] = useState(node.edge_label)
  const [attaching, setAttaching] = useState(false)
  const [draftAtt, setDraftAtt] = useState({
    kind: 'link' as AttachmentKind,
    name: '',
    gist: '',
    ref: '',
  })

  // The drafts belong to a NODE, not to the card, so a different node re-seeds
  // them. Done by comparing the id during render — React's own
  // adjust-state-on-prop-change pattern — rather than in an effect: an effect
  // keyed on `node.id` alone would be a lie to the dependency linter, and one
  // keyed honestly on `node.notes` would re-seed on every remote keystroke and
  // fight whoever is typing here.
  const [seeded, setSeeded] = useState(node.id)
  if (seeded !== node.id) {
    setSeeded(node.id)
    setNotes(node.notes)
    setEdgeLabel(node.edge_label)
    setAttaching(false)
    setDraftAtt({ kind: 'link', name: '', gist: '', ref: '' })
  }

  // Focus the title editor when it opens and select it all: the common edit is
  // replacing a first-draft thought, not appending to it.
  const titleRef = useRef<HTMLTextAreaElement | null>(null)
  useEffect(() => {
    if (editing) titleRef.current?.select?.()
  }, [editing])

  const notesRef = useRef<HTMLTextAreaElement | null>(null)
  useEffect(() => {
    if (!reveal) return
    if (reveal === 'notes') notesRef.current?.focus()
    if (reveal === 'attach') setAttaching(true)
    onRevealed()
  }, [reveal, onRevealed])

  const kindLabel: Record<MapNode['kind'], string> = {
    thought: labels.kindThought,
    question: labels.kindQuestion,
    decision: labels.kindDecision,
    screen: labels.kindScreen,
    component: labels.kindComponent,
  }
  const shapeLabel: Record<NodeShape, string> = {
    rounded: labels.shapeRounded,
    square: labels.shapeSquare,
    pill: labels.shapePill,
  }
  const attLabel: Record<AttachmentKind, string> = {
    pdf: labels.attPdf,
    code: labels.attCode,
    table: labels.attTable,
    diagram: labels.attDiagram,
    audio: labels.attAudio,
    link: labels.attLink,
  }

  const mark = KIND_MARK[node.kind]
  const context = node.attachments.length > 0 || relations.length > 0

  const titleBlock = editing ? (
    <Textarea
      ref={titleRef}
      autoFocus
      value={draft}
      onChange={(e) => onDraft(e.target.value)}
      onBlur={onCommit}
      onKeyDown={(e) => {
        // Enter commits rather than adding a line: a node is a sentence or two,
        // and the next Enter should make the next thought. Shift+Enter is the
        // escape hatch.
        if (e.key === 'Enter' && !e.shiftKey) {
          e.preventDefault()
          onCommit()
        } else if (e.key === 'Escape') {
          e.preventDefault()
          onCancel()
        }
        e.stopPropagation()
      }}
      className="min-h-0 resize-none px-1.5 py-1 text-[12.5px] leading-snug"
      rows={2}
    />
  ) : (
    <div className={cn('text-foreground text-[12.5px] leading-snug', !expanded && 'line-clamp-2')}>
      {mark ? `${mark} ` : ''}
      {node.title}
    </div>
  )

  if (!expanded) {
    return (
      <div className={cn('flex h-full flex-col justify-center gap-0.5 px-2.5 py-1.5', className)}>
        {titleBlock}
        {/* The marks. A collapsed map still says WHERE the substance is, which is
            the whole reason it is safe to draw only one card in full. */}
        <div className="text-muted-foreground flex items-center gap-1.5 truncate font-mono text-[10px]">
          {node.promoted && <span>→ {node.promoted.kind}</span>}
          {node.notes && <span title={labels.hasNotes}>≋</span>}
          {context && (
            <span title={labels.hasContext}>
              ¶ {node.attachments.length + relations.length}
            </span>
          )}
          {node.origin === 'agent' && <span>⌁</span>}
        </div>
      </div>
    )
  }

  const full = node.attachments.length >= MAX_ATTACHMENTS

  return (
    <div
      className={cn('flex h-full min-h-0 flex-col', className)}
      // Everything below the grip belongs to the card, not the canvas: a press
      // here must not start a pan, a drag or a deselect.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
      // The canvas keyboard is a map keyboard: Enter grows a branch, Space folds
      // one, Backspace prunes. None of that may happen because somebody pressed
      // Space on this card's checkbox, so every key inside the card stops here.
      // ⌘K still works — the page listens for it in the capture phase.
      onKeyDown={(e) => e.stopPropagation()}
    >
      {/* The grip. It deliberately does NOT stop propagation, so the selected
          node can still be dragged and dropped like any other. */}
      <div className="border-b-border-soft text-muted-foreground flex cursor-grab items-center gap-1.5 border-b px-2 py-1 font-mono text-[10px]">
        <span aria-hidden>⠿</span>
        <span className="min-w-0 truncate">
          {labels.origin} {node.origin === 'agent' ? labels.originAgent : labels.originHuman}
        </span>
        {node.promoted && (
          <span className="text-foreground ml-auto shrink-0">
            → {labels.promoted} {node.promoted.id}
          </span>
        )}
      </div>

      <div className="flex min-h-0 flex-col gap-2 overflow-y-auto px-2 py-2">
        <button
          type="button"
          disabled={!canWrite || editing}
          onClick={onEdit}
          className="cursor-text text-left disabled:cursor-default"
        >
          {titleBlock}
        </button>

        {!canWrite && (
          <p className="text-muted-foreground m-0 text-[11px]">{labels.readOnly}</p>
        )}

        <label className="flex flex-col gap-0.5">
          <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.notes}</span>
          <Textarea
            ref={notesRef}
            value={notes}
            disabled={!canWrite}
            maxLength={MAX_NOTES}
            rows={3}
            placeholder={labels.notesHint}
            onChange={(e) => setNotes(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
            onBlur={() => notes !== node.notes && onNotes(node.id, notes)}
            className="text-[11.5px]"
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
              value={node.kind}
              disabled={!canWrite}
              onChange={(e) => onFields(node.id, { kind: e.target.value as MapNode['kind'] })}
            >
              {NODE_KINDS.map((k) => (
                <option key={k} value={k}>
                  {kindLabel[k]}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-0.5">
            <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.shape}</span>
            <select
              className={FIELD}
              value={NODE_SHAPES.includes(node.shape as NodeShape) ? node.shape : 'rounded'}
              disabled={!canWrite}
              onChange={(e) => onFields(node.id, { shape: e.target.value })}
            >
              {NODE_SHAPES.map((s) => (
                <option key={s} value={s}>
                  {shapeLabel[s]}
                </option>
              ))}
            </select>
          </label>
        </div>

        <label className="flex flex-col gap-0.5">
          <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.edgeLabel}</span>
          <Input
            value={edgeLabel}
            disabled={!canWrite}
            placeholder={labels.edgeLabelHint}
            onChange={(e) => setEdgeLabel(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
            onBlur={() =>
              edgeLabel !== node.edge_label && onFields(node.id, { edge_label: edgeLabel })
            }
            className="h-7 text-[11.5px]"
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
                'border-border h-4 w-4 cursor-pointer rounded border text-[9px] disabled:opacity-40',
                node.color === c && 'ring-ring ring-2',
              )}
            >
              {c ? '' : '×'}
            </button>
          ))}
        </div>

        <label className="flex items-center gap-1.5 text-[11.5px]">
          <input
            type="checkbox"
            checked={node.reviewed}
            disabled={!canWrite}
            onChange={(e) => onFields(node.id, { reviewed: e.target.checked })}
          />
          <span>{labels.reviewed}</span>
        </label>

        {/* ---- attachments, as chips ---- */}
        <div className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[10.5px] font-[650]">
            {labels.attachments}
          </span>
          {node.attachments.length > 0 && (
            <ul className="m-0 flex list-none flex-wrap gap-1 p-0">
              {node.attachments.map((a) => (
                <li
                  key={a.id}
                  title={a.gist ? `${a.name} — ${a.gist}` : a.name}
                  className="border-border-soft bg-muted flex max-w-full items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10.5px]"
                >
                  <span className="min-w-0 truncate">
                    {attLabel[a.kind]} · {a.name}
                  </span>
                  <button
                    type="button"
                    disabled={!canWrite}
                    aria-label={labels.removeAttachment}
                    onClick={() => onRemoveAttachment(node.id, a.id)}
                    className="cursor-pointer disabled:opacity-40"
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
          {full ? (
            <p className="text-muted-foreground m-0 text-[10.5px]">{labels.attachmentsFull}</p>
          ) : attaching ? (
            <div className="flex flex-col gap-1">
              <select
                className={FIELD}
                aria-label={labels.attachmentKind}
                value={draftAtt.kind}
                disabled={!canWrite}
                onChange={(e) =>
                  setDraftAtt({ ...draftAtt, kind: e.target.value as AttachmentKind })
                }
              >
                {ATTACHMENT_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {attLabel[k]}
                  </option>
                ))}
              </select>
              <Input
                value={draftAtt.name}
                disabled={!canWrite}
                placeholder={labels.attachmentName}
                onKeyDown={(e) => e.stopPropagation()}
                onChange={(e) => setDraftAtt({ ...draftAtt, name: e.target.value })}
                className="h-7 text-[11.5px]"
              />
              <Input
                value={draftAtt.ref}
                disabled={!canWrite}
                placeholder={labels.attachmentRef}
                onKeyDown={(e) => e.stopPropagation()}
                onChange={(e) => setDraftAtt({ ...draftAtt, ref: e.target.value })}
                className="h-7 text-[11.5px]"
              />
              <Input
                value={draftAtt.gist}
                disabled={!canWrite}
                placeholder={labels.attachmentGist}
                onKeyDown={(e) => e.stopPropagation()}
                onChange={(e) => setDraftAtt({ ...draftAtt, gist: e.target.value })}
                className="h-7 text-[11.5px]"
              />
              <Button
                variant="outline"
                size="sm"
                disabled={!canWrite || !draftAtt.name.trim() || !draftAtt.ref.trim()}
                onClick={() => {
                  onAddAttachment(node.id, {
                    kind: draftAtt.kind,
                    name: draftAtt.name.trim(),
                    gist: draftAtt.gist.trim(),
                    ref: draftAtt.ref.trim(),
                  })
                  setDraftAtt({ kind: 'link', name: '', gist: '', ref: '' })
                  setAttaching(false)
                }}
              >
                {labels.addAttachment}
              </Button>
            </div>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              disabled={!canWrite}
              onClick={() => setAttaching(true)}
            >
              + {labels.addAttachment}
            </Button>
          )}
        </div>

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
                  <li key={r.id} className="flex items-center gap-1 text-[11px]">
                    <span className="min-w-0 grow truncate">
                      {r.from === node.id ? '→' : '←'} {titleOf.get(other) ?? other}
                      {r.label ? ` · ${r.label}` : ''}
                    </span>
                    <button
                      type="button"
                      disabled={!canWrite}
                      aria-label={labels.removeRelation}
                      onClick={() => onRemoveRelation(r.id)}
                      className="cursor-pointer disabled:opacity-40"
                    >
                      ×
                    </button>
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
