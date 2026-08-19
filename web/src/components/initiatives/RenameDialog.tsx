// Rename an initiative, from wherever you can see its name.
//
// The title has always been editable — as inline text on the open document. That
// covers renaming the thing you are reading, and nothing else: to fix a title in
// the tree you had to open that document first, losing the one you were on. A
// tree is where names are compared, so it is where a wrong one gets noticed.
//
// A modal rather than inline editing in the tree, because a tree row is 13px and
// truncated: the field you type a 300-character title into should not be the box
// that was too small to show the old one.
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

export interface RenameDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The current title. Seeds the field each time the dialog opens. */
  value: string
  /** Shown under the heading so it is clear WHICH one is being renamed. */
  subject?: string
  onRename: (title: string) => void
  onInvalid: (message: string) => void
  labels: {
    title: string
    subtitle: string
    field: string
    placeholder: string
    save: string
    cancel: string
    needTitle: string
  }
}

export function RenameDialog({
  open,
  onOpenChange,
  value,
  subject,
  onRename,
  onInvalid,
  labels,
}: RenameDialogProps) {
  const [title, setTitle] = useState(value)

  // Re-seeded on open rather than held across closes: the field should say what
  // the name IS every time it appears, including after a cancelled edit.
  useEffect(() => {
    if (open) setTitle(value)
  }, [open, value])

  function submit() {
    const next = title.trim()
    if (!next) {
      onInvalid(labels.needTitle)
      return
    }
    // A no-op rename is a close, not a request: the toast would claim a change
    // that never happened.
    if (next === value.trim()) {
      onOpenChange(false)
      return
    }
    onRename(next)
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{subject ? subject : labels.subtitle}</DialogDescription>
        </DialogHeader>

        <Field label={labels.field}>
          {(id) => (
            <Input
              id={id}
              autoFocus
              value={title}
              placeholder={labels.placeholder}
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  submit()
                }
              }}
            />
          )}
        </Field>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button onClick={submit}>{labels.save}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
