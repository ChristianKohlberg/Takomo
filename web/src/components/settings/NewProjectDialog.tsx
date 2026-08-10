// Create a project.
//
// The id is the part worth designing around: it is what every ticket id is
// prefixed with, it appears in every URL and every agent's config, and the
// server will not let it be changed afterwards. So the field is validated as it
// is typed rather than on submit, and the name defaults to the id instead of
// being required — a project called `demo` named "demo" is a perfectly good
// project, and making someone type it twice is how the field earns a typo.
import { useState } from 'react'
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

export interface NewProjectDialogLabels {
  title: string
  subtitle: string
  id: string
  idPh: string
  idHint: string
  idInvalid: string
  name: string
  namePh: string
  nameHint: string
  create: string
  cancel: string
}

export interface NewProjectDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  labels: NewProjectDialogLabels
  onCreate: (fields: { id: string; name: string }) => Promise<unknown>
}

/** Lowercase, digits and dashes — what a ticket id prefix can carry. */
export function isValidProjectId(id: string): boolean {
  return /^[a-z0-9][a-z0-9-]*$/.test(id)
}

export function NewProjectDialog({
  open,
  onOpenChange,
  labels,
  onCreate,
}: NewProjectDialogProps) {
  const [id, setId] = useState('')
  const [name, setName] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  const idBad = id.length > 0 && !isValidProjectId(id)

  function reset() {
    setId('')
    setName('')
    setErr('')
    setBusy(false)
  }

  async function submit() {
    if (!isValidProjectId(id)) {
      setErr(labels.idInvalid)
      return
    }
    setBusy(true)
    setErr('')
    try {
      await onCreate({ id, name: name.trim() || id })
      reset()
    } catch (e) {
      setErr((e as Error)?.message ?? String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) reset()
        onOpenChange(o)
      }}
    >
      <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-116">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <Field label={labels.id} hint={idBad ? labels.idInvalid : labels.idHint}>
            {(fid) => (
              <Input
                id={fid}
                value={id}
                placeholder={labels.idPh}
                aria-invalid={idBad || undefined}
                onChange={(e) => setId(e.target.value.toLowerCase())}
                className="font-mono"
                autoFocus
              />
            )}
          </Field>
          <Field label={labels.name} hint={labels.nameHint}>
            {(fid) => (
              <Input
                id={fid}
                value={name}
                placeholder={id || labels.namePh}
                onChange={(e) => setName(e.target.value)}
              />
            )}
          </Field>
          {err && <div className="text-destructive text-[12.5px]">{err}</div>}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {labels.cancel}
          </Button>
          <Button onClick={() => void submit()} disabled={busy || !id || idBad}>
            {labels.create}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
