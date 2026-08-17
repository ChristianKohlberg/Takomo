// Add or edit a person in the directory.
//
// One dialog for both, because they are the same form: a person is a handle, a
// display name, an optional email, and the projects that may hand them work.
// Splitting it would leave two places to keep in step for the sake of one field
// that stops being editable.
//
// That field is the handle, and it is fixed once the person exists. Every
// `person:<handle>` reference and every stored assignment resolves through it, so
// renaming it would silently orphan them — the display name is what changes when
// somebody's name does. The dialog says so rather than disabling a box with no
// explanation.
//
// Membership is here rather than behind a second control because it is the fact
// that decides whether a person can be handed anything at all: somebody in the
// directory with no membership is real, addressable nowhere, and looks fine.
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
import { cn } from '@/lib/utils'
import type { Project } from '@/lib/initiatives'
import type { User } from '@/lib/users'

export interface PersonDialogLabels {
  /** Header, in each mode. */
  addTitle: string
  addSubtitle: string
  editTitle: string
  editSubtitle: string
  /** Fields. */
  handle: string
  handlePh: string
  handleHint: string
  handleFixed: string
  name: string
  namePh: string
  nameHint: string
  email: string
  emailPh: string
  emailHint: string
  projects: string
  projectsHint: string
  noProjectsPicked: string
  /** Footer. */
  save: string
  add: string
  cancel: string
  /** Refusals raised before anything is sent. */
  needHandle: string
  badHandle: string
}

export interface PersonSaved {
  handle: string
  name: string
  /** `null` clears the address; absent and null differ on the wire. */
  email: string | null
  /** The membership set the person should end up with. */
  projects: string[]
}

export interface PersonDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Absent = add somebody; present = edit them, handle fixed. */
  person?: User | null
  projects: Project[]
  labels: PersonDialogLabels
  onSave: (fields: PersonSaved) => Promise<unknown>
}

/**
 * The server's handle rule, checked here so a typo is refused before a round trip
 * — deliberately the same shape as a tag handle, which is what keeps
 * `person:<handle>` a legal reference to this person.
 */
const HANDLE = /^[a-z0-9][a-z0-9._-]{0,63}$/

export function PersonDialog({
  open,
  onOpenChange,
  person,
  projects,
  labels,
  onSave,
}: PersonDialogProps) {
  const editing = !!person
  const [handle, setHandle] = useState('')
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [picked, setPicked] = useState<string[]>([])
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  // The dialog stays mounted when closed (the trap NewTokenDialog documents), so
  // the fields are seeded from the person each time it opens — otherwise editing
  // Sam after Ada would open on Ada's name.
  useEffect(() => {
    if (!open) return
    setHandle(person?.handle ?? '')
    setName(person?.name ?? '')
    setEmail(person?.email ?? '')
    setPicked([...(person?.projects ?? [])])
    setErr('')
    setBusy(false)
  }, [open, person])

  const toggle = (list: string[], v: string): string[] =>
    list.includes(v) ? list.filter((x) => x !== v) : [...list, v]

  async function submit() {
    const h = handle.trim()
    if (!editing) {
      if (!h) {
        setErr(labels.needHandle)
        return
      }
      if (!HANDLE.test(h)) {
        setErr(labels.badHandle)
        return
      }
    }
    setBusy(true)
    setErr('')
    try {
      await onSave({
        handle: editing ? person!.handle : h,
        name: name.trim(),
        // An empty box means "no address", which is a clear instruction to remove
        // one — not an absent field.
        email: email.trim() ? email.trim() : null,
        projects: [...picked],
      })
    } catch (e) {
      setErr((e as Error)?.message ?? String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[86vh] max-w-[calc(100%-2rem)] overflow-y-auto sm:max-w-124">
        <DialogHeader>
          <DialogTitle>{editing ? labels.editTitle : labels.addTitle}</DialogTitle>
          <DialogDescription>
            {editing ? labels.editSubtitle : labels.addSubtitle}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          {editing ? (
            // Shown, not offered: the handle is the identity everything resolves
            // through, and a reader needs to see which person this is.
            <Field label={labels.handle} hint={labels.handleFixed}>
              {(id) => (
                <div id={id} className="font-mono text-[13px] font-[650]">
                  {person!.handle}
                </div>
              )}
            </Field>
          ) : (
            <Field label={labels.handle} hint={labels.handleHint}>
              {(id) => (
                <Input
                  id={id}
                  value={handle}
                  placeholder={labels.handlePh}
                  onChange={(e) => setHandle(e.target.value)}
                  autoFocus
                />
              )}
            </Field>
          )}

          <Field label={labels.name} hint={labels.nameHint}>
            {(id) => (
              <Input
                id={id}
                value={name}
                placeholder={labels.namePh}
                onChange={(e) => setName(e.target.value)}
                autoFocus={editing}
              />
            )}
          </Field>

          <Field label={labels.email} hint={labels.emailHint}>
            {(id) => (
              <Input
                id={id}
                type="email"
                value={email}
                placeholder={labels.emailPh}
                onChange={(e) => setEmail(e.target.value)}
              />
            )}
          </Field>

          {projects.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
                {labels.projects}
              </span>
              <div className="flex flex-wrap gap-1.5">
                {projects.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    aria-pressed={picked.includes(p.id)}
                    onClick={() => setPicked((cur) => toggle(cur, p.id))}
                    className={cn(
                      'cursor-pointer rounded-lg border px-2.5 py-1 font-mono text-[12px] transition-colors',
                      picked.includes(p.id)
                        ? 'border-ring bg-secondary text-primary'
                        : 'border-border text-muted-foreground hover:text-foreground',
                    )}
                  >
                    {p.id}
                  </button>
                ))}
              </div>
              <span className="text-muted-foreground text-[11px]">
                {/* Nothing picked is a real state with a consequence, not an
                    absence — unlike a token's project allowlist, where none means
                    all. Saying so here is the difference between a person who
                    cannot be assigned and one who looks fine. */}
                {picked.length === 0 ? labels.noProjectsPicked : labels.projectsHint}
              </span>
            </div>
          )}

          {err && <div className="text-destructive text-[12.5px]">{err}</div>}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            {labels.cancel}
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            {editing ? labels.save : labels.add}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
