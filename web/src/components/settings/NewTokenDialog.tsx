// Mint a token.
//
// A dialog rather than the form that used to unfold inside the card: minting
// needs four decisions, and a form that pushes the list it belongs to down the
// page — while the list is the thing you consulted to decide what to mint —
// is the worst place to put it.
//
// Scopes are checkboxes, not a comma-separated text field. The old field
// accepted `admin ` with a trailing space, `Admin`, and `red` for `read`, and
// the server correctly refused none of those — they are simply scopes that grant
// nothing, so the token minted fine and failed later, somewhere else.
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
import { cn } from '@/lib/utils'
import type { Project } from '@/lib/initiatives'

/** The scopes a token can carry, with what each one buys. */
export const SCOPES = ['read', 'write', 'human', 'admin'] as const
export type Scope = (typeof SCOPES)[number]

export interface NewTokenDialogLabels {
  title: string
  subtitle: string
  actor: string
  actorPh: string
  actorHint: string
  scopes: string
  scopeRead: string
  scopeWrite: string
  scopeHuman: string
  scopeAdmin: string
  projects: string
  projectsHint: string
  allProjects: string
  create: string
  cancel: string
  needActor: string
  needScope: string
}

export interface NewTokenDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  projects: Project[]
  labels: NewTokenDialogLabels
  onCreate: (fields: {
    actor: string
    scopes: string[]
    projects?: string[] | null
  }) => Promise<unknown>
}

export function NewTokenDialog({
  open,
  onOpenChange,
  projects,
  labels,
  onCreate,
}: NewTokenDialogProps) {
  const [actor, setActor] = useState('')
  const [scopes, setScopes] = useState<Scope[]>(['read', 'write'])
  const [picked, setPicked] = useState<string[]>([])
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  const scopeLabel: Record<Scope, string> = {
    read: labels.scopeRead,
    write: labels.scopeWrite,
    human: labels.scopeHuman,
    admin: labels.scopeAdmin,
  }

  // The dialog stays mounted when closed — the same trap CreateScheduleDialog
  // documents — so without this the next open holds the last token's actor.
  function reset() {
    setActor('')
    setScopes(['read', 'write'])
    setPicked([])
    setErr('')
    setBusy(false)
  }

  const toggle = <T,>(list: T[], v: T): T[] =>
    list.includes(v) ? list.filter((x) => x !== v) : [...list, v]

  async function submit() {
    if (!actor.trim()) {
      setErr(labels.needActor)
      return
    }
    if (scopes.length === 0) {
      setErr(labels.needScope)
      return
    }
    setBusy(true)
    setErr('')
    try {
      // No project selected means EVERY project, which is `null` on the wire.
      // An empty array would be an allowlist that permits nothing.
      await onCreate({
        actor: actor.trim(),
        scopes: [...scopes],
        projects: picked.length ? picked : null,
      })
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
      <DialogContent className="max-h-[86vh] max-w-[calc(100%-2rem)] overflow-y-auto sm:max-w-124">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <Field label={labels.actor} hint={labels.actorHint}>
            {(id) => (
              <Input
                id={id}
                value={actor}
                placeholder={labels.actorPh}
                onChange={(e) => setActor(e.target.value)}
                autoFocus
              />
            )}
          </Field>

          <div className="flex flex-col gap-1.5">
            <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
              {labels.scopes}
            </span>
            <div className="flex flex-col gap-1">
              {SCOPES.map((s) => (
                <label
                  key={s}
                  className="hover:bg-muted flex cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5"
                >
                  <input
                    type="checkbox"
                    className="mt-0.5 cursor-pointer"
                    checked={scopes.includes(s)}
                    onChange={() => setScopes((cur) => toggle(cur, s))}
                  />
                  <span className="min-w-0">
                    <span className="font-mono text-[13px] font-[650]">{s}</span>
                    <span className="text-muted-foreground block text-[11.5px] leading-snug">
                      {scopeLabel[s]}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          </div>

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
                {picked.length === 0 ? labels.allProjects : labels.projectsHint}
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
            {labels.create}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
