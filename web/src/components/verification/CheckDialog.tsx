// File a check from the browser.
//
// The dialog is opinionated on purpose: the field order walks the definition of
// a check — one action, one entry precondition, one layer — because the most
// expensive mistake here is drawing the boundary at a SCREEN instead of a state
// transition, and it is made before any of the other fields matter. The hints
// say so where the decision is taken, not in a doc nobody opens.
import { useEffect, useState } from 'react'
import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  CHECK_LAYERS,
  CHECK_SEVERITIES,
  type CheckFields,
  type Layer,
  type Severity,
} from '@/lib/verification'

export interface CheckDialogLabels {
  newCheck: string
  fTitle: string
  fTitlePh: string
  fInitiative: string
  fInitiativeNone: string
  fLayer: string
  fLayerHint: string
  fSeverity: string
  fSeverityHint: string
  fPrecondition: string
  fPreconditionPh: string
  fBody: string
  fBodyPh: string
  fGlobs: string
  fGlobsPh: string
  fGlobsHint: string
  create: string
  cancel: string
}

export interface CheckDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Initiatives to file under, as `[id, title]`. */
  initiatives: { id: string; title: string }[]
  /** Preselected initiative, when the dialog was opened from a group header. */
  defaultInitiative?: string
  labels: CheckDialogLabels
  onSubmit: (fields: CheckFields) => Promise<unknown>
}

export function CheckDialog({
  open,
  onOpenChange,
  initiatives,
  defaultInitiative,
  labels,
  onSubmit,
}: CheckDialogProps) {
  const [title, setTitle] = useState('')
  const [initiative, setInitiative] = useState('')
  const [layer, setLayer] = useState<Layer>('ui')
  const [severity, setSeverity] = useState<Severity>('advisory')
  const [precondition, setPrecondition] = useState('')
  const [body, setBody] = useState('')
  const [globs, setGlobs] = useState('')
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    setTitle('')
    setInitiative(defaultInitiative ?? '')
    setLayer('ui')
    setSeverity('advisory')
    setPrecondition('')
    setBody('')
    setGlobs('')
    setError('')
  }, [open, defaultInitiative])

  async function submit() {
    setSaving(true)
    try {
      const fields: CheckFields = { title: title.trim(), layer, severity }
      if (initiative) fields.initiative = initiative
      if (precondition.trim()) fields.precondition = precondition.trim()
      if (body.trim()) fields.body = body.trim()
      const list = globs
        .split('\n')
        .map((g) => g.trim())
        .filter(Boolean)
      if (list.length) fields.globs = list
      await onSubmit(fields)
      onOpenChange(false)
    } catch (e) {
      setError((e as { message?: string })?.message || 'failed')
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] w-full max-w-[calc(100%-2rem)] overflow-y-auto md:max-w-160">
        <DialogHeader>
          <DialogTitle>{labels.newCheck}</DialogTitle>
          <DialogDescription>{labels.fLayerHint}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <Field label={labels.fTitle}>
            {(id) => (
              <Input
                id={id}
                value={title}
                placeholder={labels.fTitlePh}
                onChange={(e) => setTitle(e.target.value)}
              />
            )}
          </Field>

          <Field label={labels.fInitiative}>
            {(id) => (
              <select
                id={id}
                className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                value={initiative}
                onChange={(e) => setInitiative(e.target.value)}
              >
                <option value="">{labels.fInitiativeNone}</option>
                {initiatives.map((i) => (
                  <option key={i.id} value={i.id}>
                    {i.title}
                  </option>
                ))}
              </select>
            )}
          </Field>

          <div className="flex flex-col gap-3 md:flex-row md:[&>*]:flex-[1_1_170px]">
            <Field label={labels.fLayer}>
              {(id) => (
                <select
                  id={id}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  value={layer}
                  onChange={(e) => setLayer(e.target.value as Layer)}
                >
                  {CHECK_LAYERS.map((l) => (
                    <option key={l} value={l}>
                      {l}
                    </option>
                  ))}
                </select>
              )}
            </Field>
            <Field label={labels.fSeverity} hint={labels.fSeverityHint}>
              {(id) => (
                <select
                  id={id}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  value={severity}
                  onChange={(e) => setSeverity(e.target.value as Severity)}
                >
                  {CHECK_SEVERITIES.map((s) => (
                    <option key={s} value={s}>
                      {s}
                    </option>
                  ))}
                </select>
              )}
            </Field>
          </div>

          <Field label={labels.fPrecondition}>
            {(id) => (
              <Input
                id={id}
                value={precondition}
                placeholder={labels.fPreconditionPh}
                onChange={(e) => setPrecondition(e.target.value)}
              />
            )}
          </Field>

          <Field label={labels.fBody}>
            {(id) => (
              <Textarea
                id={id}
                rows={3}
                value={body}
                placeholder={labels.fBodyPh}
                onChange={(e) => setBody(e.target.value)}
              />
            )}
          </Field>

          <Field label={labels.fGlobs} hint={labels.fGlobsHint}>
            {(id) => (
              <Textarea
                id={id}
                rows={2}
                value={globs}
                placeholder={labels.fGlobsPh}
                onChange={(e) => setGlobs(e.target.value)}
              />
            )}
          </Field>

          {error && <p className="text-nf text-[12.5px]">{error}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button disabled={saving || !title.trim()} onClick={() => void submit()}>
            {labels.create}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
