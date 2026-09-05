// Removing a thought, and everything growing out of it.
//
// A leaf is asked about once, because losing one first-draft sentence is the
// cheapest mistake on this surface. A branch is asked about TWICE, and the second
// question is not the first one repeated: the first says how much goes, the
// second says that it goes from a canvas other people are watching right now.
// That is the whole argument for the double confirm — on a shared map the
// deletion is not private, and the person who clicks is not necessarily the
// person who wrote the branch.
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

export interface PruneDialogLabels {
  title: string
  /** First question. `{title}` is the node, `{n}` the thoughts beneath it. */
  body: string
  bodyLeaf: string
  /** Second question — a branch only. `{n}` is the count again, deliberately. */
  confirmTitle: string
  confirmBody: string
  watching: string
  next: string
  remove: string
  cancel: string
}

export interface PruneDialogProps {
  /** The node to remove, or null when the dialog is closed. */
  node: { id: string; title: string } | null
  /** How many thoughts hang beneath it. Zero makes this a single question. */
  descendants: number
  /** Other people in the map right now, by name. */
  peers: string[]
  onOpenChange: (open: boolean) => void
  onConfirm: (id: string) => void
  labels: PruneDialogLabels
}

export function PruneDialog({
  node,
  descendants,
  peers,
  onOpenChange,
  onConfirm,
  labels,
}: PruneDialogProps) {
  const [second, setSecond] = useState(false)

  // The second question belongs to one node. Opening the dialog on another must
  // start from the first, or a stray click lands on a confirmation that was
  // reasoned about for something else.
  useEffect(() => setSecond(false), [node?.id])

  if (!node) return null
  const branch = descendants > 0
  const n = String(descendants)

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{second ? labels.confirmTitle : labels.title}</DialogTitle>
          <DialogDescription>
            {second
              ? labels.confirmBody.replace('{n}', n)
              : branch
                ? labels.body.replace('{title}', node.title).replace('{n}', n)
                : labels.bodyLeaf.replace('{title}', node.title)}
          </DialogDescription>
        </DialogHeader>

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
            variant="destructive"
            onClick={() => {
              // A branch needs the second question answered too; a leaf goes now.
              if (branch && !second) {
                setSecond(true)
                return
              }
              onConfirm(node.id)
            }}
          >
            {branch && !second ? labels.next : labels.remove}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
