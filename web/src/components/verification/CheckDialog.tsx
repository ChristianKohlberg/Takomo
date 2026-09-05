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
import { Checkbox } from '@/components/ui/checkbox'
import { Picker } from '@/components/Picker'
import {
  CHECK_LAYERS,
  CHECK_SEVERITIES,
  type CheckFields,
  type Layer,
  type Severity,
} from '@/lib/verification'

export interface CheckDialogLabels {
  newCheck: string
  fEnvironments: string
  fEnvironmentsHint: string
  fEnvironmentsNone: string
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
  fNode: string
  fNodeHint: string
  fNodeNone: string
  create: string
  cancel: string
}

export interface CheckDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Initiatives to file under, as `[id, title]`. */
  initiatives: { id: string; title: string }[]
  /** Environments this project has, to declare where the check must pass. */
  environments: { id: string; slug: string }[]
  /** Preselected initiative, when the dialog was opened from a group header. */
  defaultInitiative?: string
  /** Sections of the plan this check may be filed against, as `[id, title]`. */
  nodes: { id: string; title: string }[]
  /** Preselected section, when the screen is already filtered to one. */
  defaultNode?: string
  labels: CheckDialogLabels
  onSubmit: (fields: CheckFields) => Promise<unknown>
}

export function CheckDialog({
  open,
  onOpenChange,
  initiatives,
  environments,
  defaultInitiative,
  nodes,
  defaultNode,
  labels,
  onSubmit,
}: CheckDialogProps) {
  const [title, setTitle] = useState('')
  const [initiative, setInitiative] = useState('')
  const [node, setNode] = useState('')
  const [layer, setLayer] = useState<Layer>('ui')
  const [severity, setSeverity] = useState<Severity>('advisory')
  const [precondition, setPrecondition] = useState('')
  const [body, setBody] = useState('')
  const [globs, setGlobs] = useState('')
  const [envs, setEnvs] = useState<string[]>([])
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    setTitle('')
    setInitiative(defaultInitiative ?? '')
    setNode(defaultNode ?? '')
    setLayer('ui')
    setSeverity('advisory')
    setPrecondition('')
    setBody('')
    setGlobs('')
    setEnvs([])
    setError('')
  }, [open, defaultInitiative, defaultNode])

  async function submit() {
    setSaving(true)
    try {
      const fields: CheckFields = { title: title.trim(), layer, severity }
      if (initiative) fields.initiative = initiative
      if (node) fields.node = node
      if (precondition.trim()) fields.precondition = precondition.trim()
      if (body.trim()) fields.body = body.trim()
      const list = globs
        .split('\n')
        .map((g) => g.trim())
        .filter(Boolean)
      if (list.length) fields.globs = list
      if (envs.length) fields.environments = envs
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
              <Picker
                id={id}
                value={initiative}
                onValueChange={(v) => setInitiative(v)}
                className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                options={[
                  { value: '', label: labels.fInitiativeNone },
                  ...initiatives.map((i) => ({ value: i.id, label: i.title })),
                ]}
              />
            )}
          </Field>

          {/* The part of the plan this check is about. A check filed under no
              section is legitimate — a check about no part in particular — so
              this offers "none" rather than insisting. */}
          {nodes.length > 0 && (
            <Field label={labels.fNode} hint={labels.fNodeHint}>
              {(id) => (
                <Picker
                  id={id}
                  value={node}
                  onValueChange={(v) => setNode(v)}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  options={[
                    { value: '', label: labels.fNodeNone },
                    ...nodes.map((n) => ({ value: n.id, label: n.title })),
                  ]}
                />
              )}
            </Field>
          )}

          <div className="flex flex-col gap-3 md:flex-row md:[&>*]:flex-[1_1_170px]">
            <Field label={labels.fLayer}>
              {(id) => (
                <Picker
                  id={id}
                  value={layer}
                  onValueChange={(v) => setLayer(v as Layer)}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  options={[
                    ...CHECK_LAYERS.map((l) => ({ value: l, label: l })),
                  ]}
                />
              )}
            </Field>
            <Field label={labels.fSeverity} hint={labels.fSeverityHint}>
              {(id) => (
                <Picker
                  id={id}
                  value={severity}
                  onValueChange={(v) => setSeverity(v as Severity)}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  options={[
                    ...CHECK_SEVERITIES.map((s) => ({ value: s, label: s })),
                  ]}
                />
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

          {/* Checkboxes, not a multi-select: declaring two environments is a
              deliberate choice with a real consequence — every case is then
              tracked in both — so it should take two clicks and read as two
              things, not as a list widget. */}
          <Field label={labels.fEnvironments} hint={labels.fEnvironmentsHint}>
            {(id) => (
              <div id={id} className="flex flex-wrap gap-x-4 gap-y-1.5">
                {environments.length === 0 ? (
                  <span className="text-muted-foreground text-[12.5px]">
                    {labels.fEnvironmentsNone}
                  </span>
                ) : (
                  environments.map((e) => (
                    <label
                      key={e.id}
                      className="text-foreground flex items-center gap-1.5 text-[13px]"
                    >
                      <Checkbox
                        checked={envs.includes(e.id)}
                        onCheckedChange={(ev) =>
                        setEnvs((p) =>
                        ev === true ? [...p, e.id] : p.filter((x) => x !== e.id),
                        )
                        }
                      />
                      {e.slug}
                    </label>
                  ))
                )}
              </div>
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
