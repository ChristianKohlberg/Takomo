// Deleting an initiative — the one irreversible thing this page can do.
//
// Everything else here is additive by design: entries are append-only, an
// amendment is a new entry, `parked` sets a document aside without closing it.
// So this dialog carries the weight those do not, and it earns it by saying what
// is actually at stake instead of asking "are you sure": how many entries and
// how much of it is documents, because an initiative fed for three months and
// one opened by mistake yesterday look identical in a tree row.
//
// Tickets tagged into the lane are never blocked on, but they are worth naming
// up front: the lane is about to vanish from the map, and the work under it
// does not.
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { Initiative } from '@/lib/initiatives'
import { waiting } from '@/lib/initiatives'

export interface DeleteDialogLabels {
  title: string
  /** Body sentence; `{title}` is substituted with the initiative's name. */
  body: string
  /** "12 entries, 3 of them documents" — receives both counts. */
  contents: (entries: number, attachments: number) => string
  /** Shown when something is still waiting on a person inside it. */
  stillWaiting: string
  /** Shown when tickets carry its `initiative:` tag; receives the count. */
  taggedWork: (n: number) => string
  irreversible: string
  confirm: string
  cancel: string
}

export interface DeleteDialogProps {
  /** The initiative to delete, or null when the dialog is closed. */
  initiative: Initiative | null
  onOpenChange: (open: boolean) => void
  /**
   * Perform the delete. The caller owns the request so the page can refresh its
   * list and toast in one place.
   */
  onDelete: (id: string) => Promise<void>
  /** Tickets carrying its `initiative:` tag, when the page knows the number. */
  taggedTickets?: number
  busy?: boolean
  labels: DeleteDialogLabels
}

export function DeleteDialog({
  initiative,
  onOpenChange,
  onDelete,
  taggedTickets = 0,
  busy = false,
  labels,
}: DeleteDialogProps) {
  const [pending, setPending] = useState(false)

  useEffect(() => {
    setPending(false)
  }, [initiative?.id])

  if (!initiative) return null

  const rollup = initiative.rollup
  const entries = rollup?.entries ?? 0
  const attachments = rollup?.attachments ?? 0
  const { notes, amendments } = waiting(rollup)

  const run = async () => {
    setPending(true)
    try {
      await onDelete(initiative.id)
    } finally {
      setPending(false)
    }
  }

  const working = busy || pending

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.body.replace('{title}', initiative.title)}</DialogDescription>
        </DialogHeader>

        <div className="text-[13px]">
          {entries > 0 && <p className="m-0">{labels.contents(entries, attachments)}</p>}
          {/* An unanswered question or an undecided rewrite is somebody else
              mid-conversation. Worth one line before it goes. */}
          {notes + amendments > 0 && (
            <p className="text-[color:var(--warn,#c99a3a)] mt-1.5 mb-0 font-semibold">
              {labels.stillWaiting}
            </p>
          )}
          {taggedTickets > 0 && (
            <p className="text-muted-foreground mt-1.5 mb-0">{labels.taggedWork(taggedTickets)}</p>
          )}
          <p className="text-muted-foreground mt-1.5 mb-0">{labels.irreversible}</p>
        </div>

        <DialogFooter>
          <Button variant="outline" disabled={working} onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button
            variant="destructive"
            disabled={working}
            onClick={() => {
              void run()
            }}
          >
            {labels.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
