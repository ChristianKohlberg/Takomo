// One environment, as a card.
//
// `writable` and `data_state` sit on the face rather than behind a detail view,
// because they are what somebody checks BEFORE running anything destructive —
// putting them one click away is putting them where they will not be read.
//
// The two commands are copyable: they exist to be pasted into a terminal, and a
// command you have to retype from a screenshot is a command that gets retyped
// wrong.
import { useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { Environment } from '@/lib/verification'

export interface EnvironmentCardLabels {
  archived: string
  readOnly: string
  writable: string
  bringUp: string
  teardown: string
  credentials: string
  credentialsNote: string
  dataState: string
  notes: string
  edit: string
  archive: string
  unarchive: string
}

export interface EnvironmentCardProps {
  env: Environment
  busy: boolean
  labels: EnvironmentCardLabels
  onEdit: () => void
  onArchive: () => void
  onUnarchive: () => void
}

/** A production entry should not look like a scratch box at a glance. */
function kindTone(kind: string): string {
  if (kind === 'production') return 'bg-nfbg text-nf border-nfbd'
  if (kind === 'staging') return 'bg-secondary text-secondary-foreground'
  return 'bg-muted text-muted-foreground'
}

function Copyable({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
        {label}
      </span>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard?.writeText(value)
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1200)
        }}
        title={value}
        className="bg-muted hover:bg-secondary min-w-0 cursor-pointer rounded-md px-2 py-1.5 text-left font-mono text-[12px] break-all"
      >
        {copied ? '✓' : value}
      </button>
    </div>
  )
}

export function EnvironmentCard({
  env,
  busy,
  labels,
  onEdit,
  onArchive,
  onUnarchive,
}: EnvironmentCardProps) {
  const archived = !!env.archived_at
  return (
    <div
      className={cn(
        'bg-card border-border-soft flex flex-col gap-3 rounded-[10px] border p-3.5',
        archived && 'opacity-60',
        busy && 'pointer-events-none opacity-50',
      )}
    >
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-foreground truncate text-[14px] font-[700]">{env.slug}</span>
        <Badge className={kindTone(env.kind)}>{env.kind}</Badge>
        {/* The one badge that changes what a runner may do. */}
        <Badge
          className={
            env.writable ? 'bg-muted text-muted-foreground' : 'bg-nfbg text-nf border-nfbd'
          }
        >
          {env.writable ? labels.writable : labels.readOnly}
        </Badge>
        <Badge className="bg-muted text-muted-foreground">
          {labels.dataState}: {env.data_state}
        </Badge>
        {archived && <Badge className="bg-muted text-muted-foreground">{labels.archived}</Badge>}
        <span className="grow" />
        {archived ? (
          <Button variant="outline" size="sm" onClick={onUnarchive}>
            {labels.unarchive}
          </Button>
        ) : (
          <>
            <Button variant="outline" size="sm" onClick={onEdit}>
              {labels.edit}
            </Button>
            <Button variant="outline" size="sm" onClick={onArchive}>
              {labels.archive}
            </Button>
          </>
        )}
      </div>

      {env.name !== env.slug && (
        <div className="text-muted-foreground -mt-1.5 text-[12.5px]">{env.name}</div>
      )}

      {env.base_url && (
        <a
          href={env.base_url}
          target="_blank"
          rel="noreferrer noopener"
          className="text-primary min-w-0 truncate text-[13px] font-[620]"
        >
          {env.base_url}
        </a>
      )}

      {(env.bring_up || env.teardown) && (
        <div className="grid grid-cols-1 gap-2.5 md:grid-cols-2">
          {env.bring_up && <Copyable label={labels.bringUp} value={env.bring_up} />}
          {env.teardown && <Copyable label={labels.teardown} value={env.teardown} />}
        </div>
      )}

      {env.credentials_hint && (
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {labels.credentials}
          </span>
          <span className="text-foreground font-mono text-[12px] break-all">
            {env.credentials_hint}
          </span>
          <span className="text-muted-foreground text-[11px]">{labels.credentialsNote}</span>
        </div>
      )}

      {env.notes && <p className="text-muted-foreground text-[12.5px]">{env.notes}</p>}

      {/* No "N checks run here" line: nothing links a check to an environment
          yet, so any number here would be the PROJECT's count wearing this
          environment's name. A count that reads as a relationship which does not
          exist is worse than no count. */}
    </div>
  )
}
