// Cutting the line between a thought and its parent.
//
// Nothing is removed here: the child becomes a first-ring thought and everything
// under it comes along. So why two questions, when a leaf deletion gets one?
//
// Because of how it is REACHED. Every other structural change on this canvas is a
// deliberate gesture — a drag you can abandon mid-flight, a menu you opened, a
// key you pressed with a node selected. This one is a single click on a line
// somebody was only trying to click NEAR, on a shape whose whole job is to be a
// wide easy target. A gesture that cheap needs the same two questions the branch
// deletion gets, and for the same second reason: the map is open in front of
// other people, and the person cutting is not necessarily the person who grew the
// branch.
//
// Modelled on `PruneDialog` rather than sharing it, because the shapes disagree
// where it matters: that one asks twice only for a BRANCH, and this one always
// asks twice.
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

export interface DetachDialogLabels {
  title: string
  /** First question. `{title}` is the thought, `{parent}` what it hangs off. */
  body: string
  /** Second question. `{title}` again, deliberately. */
  confirmTitle: string
  confirmBody: string
  /** `{n}` thoughts travel with it. Omitted from the sentence when it is a leaf. */
  carries: string
  /** Other people in the map right now, by name. */
  watching: string
  next: string
  detach: string
  cancel: string
}

export interface DetachDialogProps {
  /** The edge to cut, or null when the dialog is closed. */
  edge: { child: { id: string; title: string }; parentTitle: string } | null
  /** How many thoughts travel with the child. Zero is legitimate. */
  carries: number
  peers: string[]
  onOpenChange: (open: boolean) => void
  onConfirm: (childId: string) => void
  labels: DetachDialogLabels
}

export function DetachDialog({
  edge,
  carries,
  peers,
  onOpenChange,
  onConfirm,
  labels,
}: DetachDialogProps) {
  const [second, setSecond] = useState(false)

  // The second question belongs to one edge. Opening the dialog on another must
  // start from the first, or a stray click lands on a confirmation that was
  // reasoned about for a different line.
  useEffect(() => setSecond(false), [edge?.child.id])

  if (!edge) return null
  const title = edge.child.title

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{second ? labels.confirmTitle : labels.title}</DialogTitle>
          <DialogDescription>
            {second
              ? labels.confirmBody.replace('{title}', title)
              : labels.body.replace('{title}', title).replace('{parent}', edge.parentTitle)}
          </DialogDescription>
        </DialogHeader>

        {!second && carries > 0 && (
          <p className="text-muted-foreground m-0 text-[12.5px]">
            {labels.carries.replace('{n}', String(carries))}
          </p>
        )}

        {second && peers.length > 0 && (
          <p className="m-0 text-[12.5px] font-semibold text-amber-700 dark:text-amber-400">
            {labels.watching.replace('{names}', peers.join(', '))}
          </p>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button
            onClick={() => {
              if (!second) {
                setSecond(true)
                return
              }
              onConfirm(edge.child.id)
            }}
          >
            {second ? labels.detach : labels.next}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
