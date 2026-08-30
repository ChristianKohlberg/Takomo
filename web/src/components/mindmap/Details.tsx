// Everything about one node that does not fit in a box on the canvas.
//
// The canvas renders `title`, and the 280-character cap on it is the method
// rather than a limitation: a branch you can read at a glance is what makes a
// mindmap worth having. `notes` is where the argument goes instead — 8,000
// characters, never drawn on the canvas, revealed only on the node somebody has
// opened. That is the same rule relocated, not retired.
//
// The rest of this panel is the explicit-fields decision made visible. `color`,
// `shape`, `kind` and `edge_label` are separate keys in the document, so two
// people recolouring and re-labelling the same node at the same time both get
// what they asked for. A single `style` blob would have merged as a whole and
// one of them would have silently lost.
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  ATTACHMENT_KINDS,
  MAX_NOTES,
  NODE_KINDS,
  type AttachmentKind,
  type MapNode,
  type NodeFields,
  type Relationship,
} from '@/lib/mindmap-doc'

/** Node shapes. Explicit, and stored as a plain string like every other field. */
export const NODE_SHAPES = ['rounded', 'square', 'pill'] as const
export type NodeShape = (typeof NODE_SHAPES)[number]

/** A small fixed palette. A colour picker would invite 16 million answers to a
 *  question whose useful answers number about six. */
export const NODE_COLORS = ['', '#fee2e2', '#ffedd5', '#fef9c3', '#dcfce7', '#dbeafe', '#f3e8ff']

export interface DetailsLabels {
  heading: string
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
  attachmentsHint: string
  attachmentsFull: string
  attachmentKind: string
  attachmentName: string
  attachmentGist: string
  attachmentRef: string
  addAttachment: string
  removeAttachment: string
  relations: string
  relationsHint: string
  startRelation: string
  cancelRelation: string
  removeRelation: string
  noRelations: string
  promoted: string
  deleteNode: string
  readOnly: string
  /** Node kinds and attachment kinds, keyed by their stored value. */
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

export interface DetailsProps {
  node: MapNode
  /** Only the relations that touch this node. */
  relations: Relationship[]
  /** Titles for the far end of each relation, so a row reads as prose. */
  titleOf: ReadonlyMap<string, string>
  canWrite: boolean
  /** True while this node is waiting for the second click of a relation. */
  drawingRelation: boolean
  onNotes: (id: string, notes: string) => void
  onFields: (id: string, fields: Partial<NodeFields>) => void
  onStartRelation: (id: string) => void
  onCancelRelation: () => void
  onRemoveRelation: (relationId: string) => void
  onAddAttachment: (
    id: string,
    draft: { kind: AttachmentKind; name: string; gist: string; ref: string },
  ) => void
  onRemoveAttachment: (id: string, attachmentId: string) => void
  onDelete: (id: string) => void
  labels: DetailsLabels
  className?: string
}

const FIELD =
  'border-border-soft bg-card text-foreground w-full rounded-md border px-2 py-1.5 text-[12.5px]'

export function Details({
  node,
  relations,
  titleOf,
  canWrite,
  drawingRelation,
  onNotes,
  onFields,
  onStartRelation,
  onCancelRelation,
  onRemoveRelation,
  onAddAttachment,
  onRemoveAttachment,
  onDelete,
  labels,
  className,
}: DetailsProps) {
  // The notes box is a draft until it blurs. Writing every keystroke into the
  // document would be correct and unkind: it is one shared history, and a
  // paragraph typed slowly would arrive at a collaborator letter by letter.
  const [notes, setNotes] = useState(node.notes)
  const [edgeLabel, setEdgeLabel] = useState(node.edge_label)
  const [draft, setDraft] = useState({ kind: 'link' as AttachmentKind, name: '', gist: '', ref: '' })

  // The drafts belong to a NODE, not to the panel, so selecting another one
  // re-seeds them. Done by comparing the id during render — React's own
  // adjust-state-on-prop-change pattern — rather than in an effect: an effect
  // keyed on `node.id` alone would be a lie to the dependency linter, and one
  // keyed honestly on `node.notes` would re-seed on every remote keystroke and
  // fight whoever is typing here.
  const [seeded, setSeeded] = useState(node.id)
  if (seeded !== node.id) {
    setSeeded(node.id)
    setNotes(node.notes)
    setEdgeLabel(node.edge_label)
    setDraft({ kind: 'link', name: '', gist: '', ref: '' })
  }

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

  const full = node.attachments.length >= 20

  return (
    <div className={className}>
      <div className="flex flex-col gap-3 p-3">
        <div>
          <div className="text-muted-foreground text-[11px] font-[700] uppercase">
            {labels.heading}
          </div>
          <div className="text-foreground mt-0.5 text-[13.5px] leading-snug font-[680]">
            {node.title}
          </div>
          <div className="text-muted-foreground mt-0.5 font-mono text-[10.5px]">
            {node.id} · {labels.origin}{' '}
            {node.origin === 'agent' ? labels.originAgent : labels.originHuman}
            {node.promoted ? ` · ${labels.promoted} ${node.promoted.id}` : ''}
          </div>
        </div>

        {!canWrite && <p className="text-muted-foreground m-0 text-[12px]">{labels.readOnly}</p>}

        <label className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[11.5px] font-[650]">{labels.notes}</span>
          <Textarea
            value={notes}
            disabled={!canWrite}
            maxLength={MAX_NOTES}
            rows={6}
            placeholder={labels.notesHint}
            onChange={(e) => setNotes(e.target.value)}
            onBlur={() => notes !== node.notes && onNotes(node.id, notes)}
            className="text-[12.5px]"
          />
          <span className="text-muted-foreground text-[10.5px]">
            {labels.notesCount.replace('{n}', String(notes.length)).replace('{max}', String(MAX_NOTES))}
          </span>
        </label>

        <div className="grid grid-cols-2 gap-2">
          <label className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[11.5px] font-[650]">{labels.kind}</span>
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
          <label className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[11.5px] font-[650]">{labels.shape}</span>
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

        <div className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[11.5px] font-[650]">{labels.color}</span>
          <div className="flex flex-wrap gap-1.5">
            {NODE_COLORS.map((c) => (
              <button
                key={c || 'none'}
                type="button"
                disabled={!canWrite}
                aria-label={c || labels.colorNone}
                aria-pressed={node.color === c}
                onClick={() => onFields(node.id, { color: c })}
                style={c ? { backgroundColor: c } : undefined}
                className={
                  'border-border h-6 w-6 cursor-pointer rounded-md border text-[10px] disabled:opacity-40 ' +
                  (node.color === c ? 'ring-ring ring-2' : '')
                }
              >
                {c ? '' : '×'}
              </button>
            ))}
          </div>
        </div>

        <label className="flex flex-col gap-1">
          <span className="text-muted-foreground text-[11.5px] font-[650]">{labels.edgeLabel}</span>
          <Input
            value={edgeLabel}
            disabled={!canWrite}
            placeholder={labels.edgeLabelHint}
            onChange={(e) => setEdgeLabel(e.target.value)}
            onBlur={() =>
              edgeLabel !== node.edge_label && onFields(node.id, { edge_label: edgeLabel })
            }
            className="text-[12.5px]"
          />
        </label>

        <label className="flex items-center gap-2 text-[12.5px]">
          <input
            type="checkbox"
            checked={node.reviewed}
            disabled={!canWrite}
            onChange={(e) => onFields(node.id, { reviewed: e.target.checked })}
          />
          <span>{labels.reviewed}</span>
        </label>

        {/* ---- relations ---- */}
        <div className="border-border-soft flex flex-col gap-1.5 border-t pt-3">
          <div className="text-muted-foreground text-[11px] font-[700] uppercase">
            {labels.relations}
          </div>
          <p className="text-muted-foreground m-0 text-[11.5px]">{labels.relationsHint}</p>
          {relations.length === 0 ? (
            <div className="text-muted-foreground text-[12px]">{labels.noRelations}</div>
          ) : (
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {relations.map((r) => {
                const other = r.from === node.id ? r.to : r.from
                return (
                  <li key={r.id} className="flex items-center gap-1.5 text-[12px]">
                    <span className="min-w-0 grow truncate">
                      {r.from === node.id ? '→' : '←'} {titleOf.get(other) ?? other}
                      {r.label ? ` · ${r.label}` : ''}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={!canWrite}
                      aria-label={labels.removeRelation}
                      onClick={() => onRemoveRelation(r.id)}
                    >
                      ×
                    </Button>
                  </li>
                )
              })}
            </ul>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={!canWrite}
            onClick={() => (drawingRelation ? onCancelRelation() : onStartRelation(node.id))}
          >
            {drawingRelation ? labels.cancelRelation : labels.startRelation}
          </Button>
        </div>

        {/* ---- attachments ---- */}
        <div className="border-border-soft flex flex-col gap-1.5 border-t pt-3">
          <div className="text-muted-foreground text-[11px] font-[700] uppercase">
            {labels.attachments}
          </div>
          <p className="text-muted-foreground m-0 text-[11.5px]">{labels.attachmentsHint}</p>
          <ul className="m-0 flex list-none flex-col gap-1 p-0">
            {node.attachments.map((a) => (
              <li key={a.id} className="flex items-start gap-1.5 text-[12px]">
                <span className="min-w-0 grow">
                  <span className="font-[650]">
                    {attLabel[a.kind]} · {a.name}
                  </span>
                  {a.gist && <span className="text-muted-foreground block">{a.gist}</span>}
                  <span className="text-muted-foreground block truncate font-mono text-[10.5px]">
                    {a.ref}
                  </span>
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={!canWrite}
                  aria-label={labels.removeAttachment}
                  onClick={() => onRemoveAttachment(node.id, a.id)}
                >
                  ×
                </Button>
              </li>
            ))}
          </ul>
          {full ? (
            <p className="text-muted-foreground m-0 text-[11.5px]">{labels.attachmentsFull}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              <select
                className={FIELD}
                aria-label={labels.attachmentKind}
                value={draft.kind}
                disabled={!canWrite}
                onChange={(e) => setDraft({ ...draft, kind: e.target.value as AttachmentKind })}
              >
                {ATTACHMENT_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {attLabel[k]}
                  </option>
                ))}
              </select>
              <Input
                value={draft.name}
                disabled={!canWrite}
                placeholder={labels.attachmentName}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                className="text-[12.5px]"
              />
              <Input
                value={draft.gist}
                disabled={!canWrite}
                placeholder={labels.attachmentGist}
                onChange={(e) => setDraft({ ...draft, gist: e.target.value })}
                className="text-[12.5px]"
              />
              <Input
                value={draft.ref}
                disabled={!canWrite}
                placeholder={labels.attachmentRef}
                onChange={(e) => setDraft({ ...draft, ref: e.target.value })}
                className="text-[12.5px]"
              />
              <Button
                variant="outline"
                size="sm"
                disabled={!canWrite || !draft.name.trim() || !draft.ref.trim()}
                onClick={() => {
                  onAddAttachment(node.id, {
                    kind: draft.kind,
                    name: draft.name.trim(),
                    gist: draft.gist.trim(),
                    ref: draft.ref.trim(),
                  })
                  setDraft({ kind: 'link', name: '', gist: '', ref: '' })
                }}
              >
                {labels.addAttachment}
              </Button>
            </div>
          )}
        </div>

        <div className="border-border-soft border-t pt-3">
          <Button
            variant="destructive"
            size="sm"
            disabled={!canWrite}
            onClick={() => onDelete(node.id)}
          >
            {labels.deleteNode}
          </Button>
        </div>
      </div>
    </div>
  )
}
