// Register or edit an environment.
//
// One dialog for both, because the fields are the same set minus one: the slug
// is immutable, so editing hides it rather than offering a control that would
// have to be refused. Checks and tool calls address an environment by its slug,
// and renaming one would break every reference silently.
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
  ENVIRONMENT_DATA_STATES,
  ENVIRONMENT_KINDS,
  type Environment,
  type EnvironmentFields,
  type EnvironmentKind,
} from '@/lib/verification'

export interface EnvironmentDialogLabels {
  newEnvironment: string
  edit: string
  fSlug: string
  fSlugHint: string
  fName: string
  fKind: string
  fBaseUrl: string
  fBaseUrlPh: string
  fBringUp: string
  fBringUpPh: string
  fBringUpHint: string
  fTeardown: string
  fTeardownPh: string
  fDataState: string
  fWritable: string
  fWritableHint: string
  fCredentials: string
  fCredentialsPh: string
  fCredentialsHint: string
  fNotes: string
  fNotesPh: string
  create: string
  save: string
  cancel: string
  secretRefused: string
}

export interface EnvironmentDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** null = registering a new one. */
  existing: Environment | null
  labels: EnvironmentDialogLabels
  onSubmit: (fields: EnvironmentFields) => Promise<unknown>
}

const EMPTY = {
  slug: '',
  name: '',
  kind: 'other' as EnvironmentKind,
  base_url: '',
  bring_up: '',
  teardown: '',
  data_state: 'unknown',
  writable: true,
  credentials_hint: '',
  notes: '',
}

export function EnvironmentDialog({
  open,
  onOpenChange,
  existing,
  labels,
  onSubmit,
}: EnvironmentDialogProps) {
  const [f, setF] = useState({ ...EMPTY })
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    setError('')
    setF(
      existing
        ? {
            slug: existing.slug,
            name: existing.name,
            kind: existing.kind,
            base_url: existing.base_url ?? '',
            bring_up: existing.bring_up,
            teardown: existing.teardown,
            data_state: existing.data_state,
            writable: existing.writable,
            credentials_hint: existing.credentials_hint ?? '',
            notes: existing.notes,
          }
        : { ...EMPTY },
    )
  }, [open, existing])

  // Production errs toward refusing a destructive run. A default, not a rule —
  // the checkbox stays editable — but it should be off unless somebody says so.
  function setKind(kind: EnvironmentKind) {
    setF((p) => ({ ...p, kind, writable: kind === 'production' ? false : p.writable }))
  }

  async function submit() {
    // Refused here as well as server-side, because the round trip is what would
    // have leaked: a pasted key travels over the wire and may land in a log
    // before the 422 comes back.
    if (f.credentials_hint.includes('-----BEGIN')) {
      setError(labels.secretRefused)
      return
    }
    setSaving(true)
    try {
      const fields: EnvironmentFields = { slug: f.slug.trim() }
      if (f.name.trim()) fields.name = f.name.trim()
      fields.kind = f.kind
      if (f.base_url.trim()) fields.base_url = f.base_url.trim()
      fields.bring_up = f.bring_up
      fields.teardown = f.teardown
      fields.data_state = f.data_state
      fields.writable = f.writable
      if (f.credentials_hint.trim()) fields.credentials_hint = f.credentials_hint.trim()
      fields.notes = f.notes
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
          <DialogTitle>{existing ? labels.edit : labels.newEnvironment}</DialogTitle>
          <DialogDescription>{labels.fBringUpHint}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          {!existing && (
            <Field label={labels.fSlug} hint={labels.fSlugHint}>
              {(id) => (
                <Input
                  id={id}
                  value={f.slug}
                  placeholder="staging"
                  onChange={(e) => setF((p) => ({ ...p, slug: e.target.value }))}
                />
              )}
            </Field>
          )}

          <div className="flex flex-col gap-3 md:flex-row md:[&>*]:flex-[1_1_170px]">
            <Field label={labels.fName}>
              {(id) => (
                <Input
                  id={id}
                  value={f.name}
                  onChange={(e) => setF((p) => ({ ...p, name: e.target.value }))}
                />
              )}
            </Field>
            <Field label={labels.fKind}>
              {(id) => (
                <select
                  id={id}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  value={f.kind}
                  onChange={(e) => setKind(e.target.value as EnvironmentKind)}
                >
                  {ENVIRONMENT_KINDS.map((k) => (
                    <option key={k} value={k}>
                      {k}
                    </option>
                  ))}
                </select>
              )}
            </Field>
            <Field label={labels.fDataState}>
              {(id) => (
                <select
                  id={id}
                  className="border-border bg-card h-9 rounded-md border px-2 text-[13px]"
                  value={f.data_state}
                  onChange={(e) => setF((p) => ({ ...p, data_state: e.target.value }))}
                >
                  {ENVIRONMENT_DATA_STATES.map((d) => (
                    <option key={d} value={d}>
                      {d}
                    </option>
                  ))}
                </select>
              )}
            </Field>
          </div>

          <Field label={labels.fBaseUrl}>
            {(id) => (
              <Input
                id={id}
                value={f.base_url}
                placeholder={labels.fBaseUrlPh}
                onChange={(e) => setF((p) => ({ ...p, base_url: e.target.value }))}
              />
            )}
          </Field>

          <div className="flex flex-col gap-3 md:flex-row md:[&>*]:flex-[1_1_170px]">
            <Field label={labels.fBringUp}>
              {(id) => (
                <Input
                  id={id}
                  value={f.bring_up}
                  placeholder={labels.fBringUpPh}
                  onChange={(e) => setF((p) => ({ ...p, bring_up: e.target.value }))}
                />
              )}
            </Field>
            <Field label={labels.fTeardown}>
              {(id) => (
                <Input
                  id={id}
                  value={f.teardown}
                  placeholder={labels.fTeardownPh}
                  onChange={(e) => setF((p) => ({ ...p, teardown: e.target.value }))}
                />
              )}
            </Field>
          </div>

          <Field label={labels.fCredentials} hint={labels.fCredentialsHint}>
            {(id) => (
              <Input
                id={id}
                value={f.credentials_hint}
                placeholder={labels.fCredentialsPh}
                onChange={(e) => setF((p) => ({ ...p, credentials_hint: e.target.value }))}
              />
            )}
          </Field>

          <label className="text-foreground flex items-start gap-2 text-[13px]">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={f.writable}
              onChange={(e) => setF((p) => ({ ...p, writable: e.target.checked }))}
            />
            <span>
              {labels.fWritable}
              <span className="text-muted-foreground block text-[11.5px]">
                {labels.fWritableHint}
              </span>
            </span>
          </label>

          <Field label={labels.fNotes}>
            {(id) => (
              <Textarea
                id={id}
                rows={3}
                value={f.notes}
                placeholder={labels.fNotesPh}
                onChange={(e) => setF((p) => ({ ...p, notes: e.target.value }))}
              />
            )}
          </Field>

          {error && <p className="text-nf text-[12.5px]">{error}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button disabled={saving || (!existing && !f.slug.trim())} onClick={() => void submit()}>
            {existing ? labels.save : labels.create}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
