// Project settings: the conventions every agent on this project reads.
//
// Save is `aria-disabled`, not `disabled` — the same pattern as the inbox's
// primary. A control that is silently inert explains nothing; this one stays
// focusable, carries its reason, and re-states it when pressed. It never
// attempts a PUT the server is already known to refuse.
import { useState } from 'react'
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
import { cn } from '@/lib/utils'
import { STYLE_MAX, saveBlockReason, type ProjectSettings } from '@/lib/project-settings'

export interface SettingsSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  settings: ProjectSettings
  onChange: (patch: Partial<ProjectSettings>) => void
  /** No `admin` scope: everything is shown, nothing can be saved. */
  readOnly: boolean
  saving: boolean
  saved: boolean
  error?: string
  onSave: () => void
  labels: {
    title: string
    subtitle: string
    langLabel: string
    langHelp: string
    langPh: string
    styleLabel: string
    styleHelp: string
    stylePh: string
    ttlLabel: string
    ttlHelp: string
    claimTtlLabel: string
    claimTtlHelp: string
    maxClaimTtlLabel: string
    maxClaimTtlHelp: string
    chars: string
    over: string
    save: string
    saving: string
    savedMsg: string
    cancel: string
    readOnlyMsg: string
  }
}

export function SettingsSheet({
  open,
  onOpenChange,
  settings: s,
  onChange,
  readOnly,
  saving,
  saved,
  error,
  onSave,
  labels,
}: SettingsSheetProps) {
  const [pressedWhileBlocked, setPressedWhileBlocked] = useState(false)
  const reason = saveBlockReason(s, readOnly, {
    readOnly: labels.readOnlyMsg,
    over: labels.over,
  })
  const used = s.style.trim().length

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[86vh] max-w-160 overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <Field label={labels.langLabel} hint={labels.langHelp}>
            {(id) => (
              <Input
                id={id}
                placeholder={labels.langPh}
                value={s.language}
                readOnly={readOnly}
                onChange={(e) => onChange({ language: e.target.value })}
              />
            )}
          </Field>

          <Field label={labels.styleLabel} hint={labels.styleHelp}>
            {(id) => (
              <>
                <Textarea
                  id={id}
                  className="min-h-28"
                  placeholder={labels.stylePh}
                  value={s.style}
                  readOnly={readOnly}
                  onChange={(e) => onChange({ style: e.target.value })}
                />
                <div
                  className={cn(
                    'text-muted-foreground text-right font-mono text-[11px]',
                    used > STYLE_MAX && 'text-destructive font-bold',
                  )}
                >
                  {labels.chars.replace('{n}', String(used)).replace('{max}', String(STYLE_MAX))}
                </div>
              </>
            )}
          </Field>

          <Field label={labels.ttlLabel} hint={labels.ttlHelp}>
            {(id) => (
              <Input
                id={id}
                type="number"
                min={0}
                value={s.ttl}
                readOnly={readOnly}
                onChange={(e) => onChange({ ttl: e.target.value })}
              />
            )}
          </Field>

          {/* The lease pair is saved in ONE call even when only one half moved —
              the endpoint validates them together. */}
          <div className="flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
            <Field label={labels.claimTtlLabel} hint={labels.claimTtlHelp}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={0}
                  value={s.claimTtl}
                  readOnly={readOnly}
                  onChange={(e) => onChange({ claimTtl: e.target.value })}
                />
              )}
            </Field>
            <Field label={labels.maxClaimTtlLabel} hint={labels.maxClaimTtlHelp}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={0}
                  value={s.maxClaimTtl}
                  readOnly={readOnly}
                  onChange={(e) => onChange({ maxClaimTtl: e.target.value })}
                />
              )}
            </Field>
          </div>

          <div className="min-h-4 text-[12.5px]">
            {error ? (
              <span className="text-destructive">{error}</span>
            ) : saved ? (
              <span className="text-ok">{labels.savedMsg}</span>
            ) : (
              (pressedWhileBlocked || readOnly) && reason && (
                <span className="text-destructive">{reason}</span>
              )
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button
            aria-disabled={reason ? 'true' : 'false'}
            className={cn(reason && 'opacity-55')}
            onClick={() => {
              if (reason) {
                // Acknowledge the press by re-stating why, rather than doing
                // nothing at all.
                setPressedWhileBlocked(true)
                return
              }
              onSave()
            }}
          >
            {saving ? labels.saving : labels.save}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
