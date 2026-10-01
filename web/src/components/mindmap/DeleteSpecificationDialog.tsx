// Deleting a project's specification: two steps and a typed confirmation.
//
// The map is the project's specification, so one `window.confirm`
// was one mis-click away from losing every section. This mirrors "Reset a
// document" in Settings: a warning, then a second step where the person types
// the specification's id or title before the button arms. The server asks for
// the same thing — the admin scope and `confirm_id` — so the dialog is the
// human half of a check that exists either way.
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

export interface DeleteSpecificationLabels {
  first: string
  second: string
  warning: string
  irreversible: string
  continue: string
  /** The label on the typed confirmation. */
  typeToConfirm: string
  final: string
  busy: string
  cancel: string
}

export interface DeleteSpecificationDialogProps {
  /** The specification to delete, or null while closed. */
  target: { id: string; title: string; project: string } | null
  onOpenChange: (open: boolean) => void
  /** Performs the deletion; the dialog passes the confirmed id. */
  onConfirm: (id: string) => Promise<void>
  labels: DeleteSpecificationLabels
}

/** Whether what was typed names this specification: its id, or its title. */
export function confirmsSpecification(typed: string, target: { id: string; title: string }): boolean {
  const value = typed.trim()
  return value.length > 0 && (value === target.id || value === target.title.trim())
}

export function DeleteSpecificationDialog({
  target,
  onOpenChange,
  onConfirm,
  labels,
}: DeleteSpecificationDialogProps) {
  const [step, setStep] = useState<1 | 2>(1)
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  // A different target (or closing) starts over, so a confirmation typed for
  // one specification can never arm the button for another.
  const [seeded, setSeeded] = useState<string | null>(target?.id ?? null)
  if (seeded !== (target?.id ?? null)) {
    setSeeded(target?.id ?? null)
    setStep(1)
    setTyped('')
    setError('')
  }

  if (!target) return null
  const armed = confirmsSpecification(typed, target)

  const close = () => {
    if (busy) return
    onOpenChange(false)
  }
  const remove = async () => {
    if (step !== 2 || !armed || busy) return
    setBusy(true)
    setError('')
    try {
      await onConfirm(target.id)
      onOpenChange(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => (open ? onOpenChange(true) : close())}>
      <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-116">
        <DialogHeader>
          <DialogTitle>{step === 1 ? labels.first : labels.second}</DialogTitle>
          <DialogDescription asChild>
            <div className="space-y-3">
              <p className="text-foreground font-semibold break-words">{target.title}</p>
              <p className="font-mono text-xs break-all">
                {target.project} / {target.id}
              </p>
              <p>{labels.warning}</p>
              <p>{labels.irreversible}</p>
            </div>
          </DialogDescription>
        </DialogHeader>
        {step === 2 && (
          <Field label={labels.typeToConfirm}>
            {(id) => (
              <Input
                id={id}
                autoComplete="off"
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void remove()
                }}
                disabled={busy}
              />
            )}
          </Field>
        )}
        {error && (
          <p role="alert" className="text-destructive text-[13px]">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="ghost" onClick={close} disabled={busy}>
            {labels.cancel}
          </Button>
          {step === 1 ? (
            <Button variant="destructive" onClick={() => setStep(2)}>
              {labels.continue}
            </Button>
          ) : (
            <Button variant="destructive" disabled={busy || !armed} onClick={() => void remove()}>
              {busy ? labels.busy : labels.final}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
