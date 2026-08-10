// A confirmation the page owns, replacing `window.confirm`.
//
// `window.confirm` was wrong here for three reasons, and the third is the one
// that mattered: it is unstyled OS chrome in the middle of a designed product,
// it cannot be localized (the browser supplies "OK"/"Cancel" in ITS language,
// not the DE/EN the rest of the page just switched to), and it blocks the whole
// event loop — which the repo already bans elsewhere, because a modal dialog
// freezes the extension driving the page.
//
// It also could not say what it was about to destroy in any detail. Revoking a
// token and deleting a project are not the same size of mistake, and a dialog
// that names the thing and colours the button by consequence is the cheapest
// possible guard against doing the second while meaning the first.
import { useState, type ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: ReactNode
  confirmLabel: string
  cancelLabel: string
  /** `destructive` colours the primary; use it for anything that removes data. */
  tone?: 'default' | 'destructive'
  onConfirm: () => Promise<unknown> | unknown
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  cancelLabel,
  tone = 'destructive',
  onConfirm,
}: ConfirmDialogProps) {
  const [busy, setBusy] = useState(false)

  async function run() {
    setBusy(true)
    try {
      await onConfirm()
      onOpenChange(false)
    } finally {
      // Cleared even on failure: the dialog stays open so the toast explaining
      // the failure is readable next to the thing it refers to, and pressing
      // again has to be possible.
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-116">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button
            variant={tone === 'destructive' ? 'destructive' : 'default'}
            onClick={() => void run()}
            disabled={busy}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
