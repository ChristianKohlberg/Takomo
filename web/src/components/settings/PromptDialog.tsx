// Ask for one string, in the product's own chrome.
//
// The sibling of ConfirmDialog, and it exists for the same three reasons
// `window.confirm` was removed: OS chrome is unstyled in a designed product, it
// cannot be localized (the browser supplies OK/Cancel in ITS language, not the
// DE/EN the page is set to), and it blocks the event loop — which stops the
// browser automation this repo is driven by dead.
//
// `window.prompt` was briefly used here for "Save to library…" and "Rename",
// which reintroduced exactly what ConfirmDialog was written to get rid of.
import { useEffect, useState } from 'react'
import { Field } from '@/components/Field'
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

export interface PromptDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  label: string
  /** Pre-filled value — a rename starts from the current name. */
  initial?: string
  confirmLabel: string
  cancelLabel: string
  onSubmit: (value: string) => Promise<unknown> | unknown
}

export function PromptDialog({
  open,
  onOpenChange,
  title,
  description,
  label,
  initial = '',
  confirmLabel,
  cancelLabel,
  onSubmit,
}: PromptDialogProps) {
  const [value, setValue] = useState(initial)
  const [busy, setBusy] = useState(false)

  // The dialog stays mounted between opens, so without this the next one holds
  // the last value — and for a rename that means offering the previous entry's
  // name as the new one.
  useEffect(() => {
    if (open) setValue(initial)
  }, [open, initial])

  async function submit() {
    const v = value.trim()
    if (!v) return
    setBusy(true)
    try {
      await onSubmit(v)
      onOpenChange(false)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-116">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <Field label={label}>
          {(id) => (
            <Input
              id={id}
              value={value}
              autoFocus
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  void submit()
                }
              }}
            />
          )}
        </Field>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button onClick={() => void submit()} disabled={busy || !value.trim()}>
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
