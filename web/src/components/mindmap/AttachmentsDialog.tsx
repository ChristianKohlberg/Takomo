// Everything hanging off one thought, in one place.
//
// This is where the attachment chips and the inline add row that used to live
// inside the expanded card went. The card is the thought; a list of pointers to
// other things is not the thought, and it was the part of the card that grew
// without bound — twenty chips and a four-field form inside a 300×320 box that
// is already drawn OVER its neighbours.
//
// So the node keeps a count badge, which is the part worth seeing from across
// the map, and the list lives here. That also gives an attachment something it
// never had: a way to be CORRECTED. A dropped file records a guessed kind and a
// name and no ref at all, which is right for a gesture that takes no typing and
// wrong to leave permanent.
import { useState } from 'react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  ATTACHMENT_KINDS,
  MAX_ATTACHMENTS,
  type Attachment,
  type AttachmentKind,
} from '@/lib/mindmap-doc'
import { cn } from '@/lib/utils'

export interface AttachmentsDialogLabels {
  /** `{title}` is the thought the attachments hang off. */
  title: string
  subtitle: string
  empty: string
  /** `{n}` of `{max}`. */
  count: string
  full: string
  kind: string
  name: string
  gist: string
  ref: string
  add: string
  addOpen: string
  edit: string
  save: string
  remove: string
  cancel: string
  close: string
  readOnly: string
  /** One per kind, so a pointer says what sort of thing it points at. */
  kinds: Record<AttachmentKind, string>
}

export interface AttachmentDraftValue {
  kind: AttachmentKind
  name: string
  gist: string
  ref: string
}

export interface AttachmentsDialogProps {
  /** The thought whose attachments these are, or null while closed. */
  node: { id: string; title: string; attachments: readonly Attachment[] } | null
  canWrite: boolean
  onOpenChange: (open: boolean) => void
  onAdd: (nodeId: string, draft: AttachmentDraftValue) => void
  onUpdate: (nodeId: string, attachmentId: string, draft: AttachmentDraftValue) => void
  onRemove: (nodeId: string, attachmentId: string) => void
  labels: AttachmentsDialogLabels
}

const EMPTY: AttachmentDraftValue = { kind: 'link', name: '', gist: '', ref: '' }

const FIELD =
  'border-border-soft bg-card text-foreground w-full rounded-md border px-2 py-1.5 text-[12px]'

export function AttachmentsDialog({
  node,
  canWrite,
  onOpenChange,
  onAdd,
  onUpdate,
  onRemove,
  labels,
}: AttachmentsDialogProps) {
  // `null` is closed, `''` is the add form, anything else is that attachment
  // being corrected. One editor at a time, because two open forms in a list this
  // short is a way to save the wrong one.
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState<AttachmentDraftValue>(EMPTY)

  // The editor belongs to a NODE. React's adjust-state-on-prop-change pattern,
  // the same one the card uses: an effect keyed on the id would be a lie to the
  // dependency linter, and one keyed honestly would fight whoever is typing.
  const [seeded, setSeeded] = useState<string | null>(node?.id ?? null)
  if (seeded !== (node?.id ?? null)) {
    setSeeded(node?.id ?? null)
    setEditing(null)
    setDraft(EMPTY)
  }

  if (!node) return null

  const items = node.attachments
  const full = items.length >= MAX_ATTACHMENTS
  const valid = draft.name.trim().length > 0

  const startAdd = () => {
    setEditing('')
    setDraft(EMPTY)
  }
  const startEdit = (a: Attachment) => {
    setEditing(a.id)
    setDraft({ kind: a.kind, name: a.name, gist: a.gist, ref: a.ref })
  }
  const save = () => {
    const value: AttachmentDraftValue = {
      kind: draft.kind,
      name: draft.name.trim(),
      gist: draft.gist.trim(),
      ref: draft.ref.trim(),
    }
    if (editing) onUpdate(node.id, editing, value)
    else onAdd(node.id, value)
    setEditing(null)
    setDraft(EMPTY)
  }

  const form = (
    <div className="border-border-soft flex flex-col gap-1.5 rounded-md border p-2">
      <label className="flex flex-col gap-0.5">
        <span className="text-muted-foreground text-[10.5px] font-[650]">{labels.kind}</span>
        <select
          className={FIELD}
          aria-label={labels.kind}
          value={draft.kind}
          onChange={(e) => setDraft({ ...draft, kind: e.target.value as AttachmentKind })}
        >
          {ATTACHMENT_KINDS.map((k) => (
            <option key={k} value={k}>
              {labels.kinds[k]}
            </option>
          ))}
        </select>
      </label>
      <Input
        aria-label={labels.name}
        placeholder={labels.name}
        value={draft.name}
        onChange={(e) => setDraft({ ...draft, name: e.target.value })}
        className="h-8 text-[12px]"
      />
      <Input
        aria-label={labels.ref}
        placeholder={labels.ref}
        value={draft.ref}
        onChange={(e) => setDraft({ ...draft, ref: e.target.value })}
        className="h-8 text-[12px]"
      />
      <Input
        aria-label={labels.gist}
        placeholder={labels.gist}
        value={draft.gist}
        onChange={(e) => setDraft({ ...draft, gist: e.target.value })}
        className="h-8 text-[12px]"
      />
      <div className="flex gap-1.5">
        <Button size="sm" disabled={!valid} onClick={save}>
          {editing ? labels.save : labels.add}
        </Button>
        <Button size="sm" variant="outline" onClick={() => setEditing(null)}>
          {labels.cancel}
        </Button>
      </div>
    </div>
  )

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{labels.title.replace('{title}', node.title)}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex max-h-[50vh] flex-col gap-1.5 overflow-y-auto">
          {items.length === 0 && (
            <p className="text-muted-foreground m-0 text-[12px]">{labels.empty}</p>
          )}
          {items.map((a) => (
            <div
              key={a.id}
              className={cn(
                'border-border-soft flex flex-col gap-1.5 rounded-md border p-2',
                editing === a.id && 'ring-ring ring-2',
              )}
            >
              {editing === a.id ? (
                form
              ) : (
                <div className="flex items-start gap-2">
                  <span className="text-muted-foreground shrink-0 font-mono text-[10.5px]">
                    {labels.kinds[a.kind]}
                  </span>
                  <div className="min-w-0 grow">
                    <div className="text-foreground truncate text-[12.5px] font-[650]">
                      {a.name}
                    </div>
                    {a.ref && (
                      <div className="text-muted-foreground truncate font-mono text-[10.5px]">
                        {a.ref}
                      </div>
                    )}
                    {a.gist && (
                      <div className="text-muted-foreground text-[11.5px] leading-snug">
                        {a.gist}
                      </div>
                    )}
                  </div>
                  {canWrite && (
                    <div className="flex shrink-0 gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`${labels.edit} — ${a.name}`}
                        onClick={() => startEdit(a)}
                      >
                        {labels.edit}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`${labels.remove} — ${a.name}`}
                        onClick={() => onRemove(node.id, a.id)}
                      >
                        ×
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}

          {canWrite && editing === '' && form}
        </div>

        <p className="text-muted-foreground m-0 text-[11px]">
          {!canWrite
            ? labels.readOnly
            : full
              ? labels.full.replace('{max}', String(MAX_ATTACHMENTS))
              : labels.count
                  .replace('{n}', String(items.length))
                  .replace('{max}', String(MAX_ATTACHMENTS))}
        </p>

        <DialogFooter>
          {canWrite && editing === null && (
            <Button variant="outline" disabled={full} onClick={startAdd}>
              + {labels.addOpen}
            </Button>
          )}
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
