// One project's configuration.
//
// These five fields lived in a dialog on /board until now, which put project
// configuration in two places: the conventions here, and everything else in
// /settings. A board is for looking at tickets; the place you go to change how a
// project behaves is the settings page. So the dialog is gone and the board's
// gear links here.
//
// The save semantics are worth keeping intact rather than rewriting: it saves
// only what changed (`saveProjectSettings`), the claim/max-claim pair goes in ONE
// call even when one half moved because the endpoint validates them together,
// and a save that changed nothing closes without claiming "Saved" — a small lie
// over an untouched form teaches the reader to distrust every later message.
import { useState, type ReactNode } from 'react'
import { ArrowLeft } from 'lucide-react'

import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import {
  STYLE_MAX,
  saveBlockReason,
  type ProjectSettings,
} from '@/lib/project-settings'

export interface ProjectDetailLabels {
  back: string
  workflowLabel: string
  langLabel: string
  langHelp: string
  langPh: string
  styleLabel: string
  styleHelp: string
  stylePh: string
  chars: string
  ttlLabel: string
  ttlHelp: string
  claimTtlLabel: string
  claimTtlHelp: string
  maxClaimTtlLabel: string
  maxClaimTtlHelp: string
  save: string
  saving: string
  savedMsg: string
  readOnlyMsg: string
  over: string
  delete: string
  archived: string
  archivedBanner: string
  archive: string
  unarchive: string
}

export interface ProjectDetailProps {
  project: { id: string; name?: string; workflow?: string; archived?: boolean }
  /**
   * The workflow editor, rendered below the conventions.
   *
   * A slot rather than props: the editor owns a draft, a debounced server
   * validation and a preflight, and threading all of that through this
   * component would make it the editor's controller instead of a form.
   */
  workflowSlot?: ReactNode
  settings: ProjectSettings
  onChange: (patch: Partial<ProjectSettings>) => void
  /** No `admin` scope: everything is shown, nothing can be saved. */
  readOnly: boolean
  saving: boolean
  saved: boolean
  error?: string
  onSave: () => void
  onBack: () => void
  onDelete: () => void
  /** Archive the project, or — when it already is — put it back to work. */
  onToggleArchive: () => void
  labels: ProjectDetailLabels
}

export function ProjectDetail({
  project,
  workflowSlot,
  settings: s,
  onChange,
  readOnly,
  saving,
  saved,
  error,
  onSave,
  onBack,
  onDelete,
  onToggleArchive,
  labels,
}: ProjectDetailProps) {
  const [pressedWhileBlocked, setPressedWhileBlocked] = useState(false)
  // An archived project is read-only here for the same reason it is read-only
  // everywhere: the server refuses these writes. Showing an editable form that
  // could only fail on save would teach the reader the gate is negotiable.
  const frozen = project.archived === true
  // `readOnly` arrives meaning "no admin scope". Keep that distinct from the
  // freeze: the form is read-only for either reason, but only an admin gets the
  // buttons at all.
  const canAdmin = !readOnly
  readOnly = readOnly || frozen
  const reason = saveBlockReason(s, readOnly, {
    readOnly: labels.readOnlyMsg,
    over: labels.over,
  })
  const used = s.style.trim().length

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft />
          {labels.back}
        </Button>
        <span className="font-mono text-[15px] font-[750]">{project.id}</span>
        {project.name && project.name !== project.id && (
          <span className="text-muted-foreground text-[13px]">{project.name}</span>
        )}
        {project.workflow && (
          <Badge variant="outline" title={labels.workflowLabel}>
            {project.workflow}
          </Badge>
        )}
        {frozen && <Badge variant="secondary">{labels.archived}</Badge>}
      </div>

      {frozen && (
        <div className="border-border-soft bg-muted text-muted-foreground rounded-xl border px-3.5 py-3 text-[13px]">
          {labels.archivedBanner}
        </div>
      )}

      <div className="flex flex-col gap-4">
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
            the endpoint validates them together, and sending half of an invalid
            pair would 422 naming a number the admin never touched. */}
        <div className="flex flex-wrap gap-3 [&>*]:flex-[1_1_170px]">
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

        <div className="flex flex-wrap items-center gap-2">
          {/* aria-disabled, not disabled: a control that is silently inert
              explains nothing. This one stays focusable, carries its reason, and
              re-states it when pressed. */}
          <Button
            aria-disabled={reason ? 'true' : 'false'}
            className={cn(reason && 'opacity-55')}
            onClick={() => {
              if (reason) {
                setPressedWhileBlocked(true)
                return
              }
              onSave()
            }}
          >
            {saving ? labels.saving : labels.save}
          </Button>
          <span className="grow" />
          {/* Archive sits BEFORE delete and stays available on a frozen project
              — it is the undo. Delete is hidden while archived: reaching for the
              irreversible one should mean leaving the gate first, deliberately. */}
          {canAdmin && (
            <Button variant="secondary" size="sm" onClick={onToggleArchive}>
              {frozen ? labels.unarchive : labels.archive}
            </Button>
          )}
          {canAdmin && !frozen && (
            <Button variant="destructive" size="sm" onClick={onDelete}>
              {labels.delete}
            </Button>
          )}
        </div>
      </div>

      {workflowSlot && (
        <>
          <hr className="border-border-soft" />
          {workflowSlot}
        </>
      )}
    </div>
  )
}
