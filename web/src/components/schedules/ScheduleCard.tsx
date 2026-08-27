// One schedule, as a row.
//
// Rows, not columns: the board sorts by state into columns, but a schedule's
// content is a HISTORY, so forcing cadences into columns would throw away the
// axis that carries all the meaning.
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Markdown } from '@/components/Markdown'
import { OccurrenceStrip } from './OccurrenceStrip'
import { cn } from '@/lib/utils'
import { cadenceText, fmtWhen, outcomeOf, type CadenceWords } from '@/lib/cadence'
import type { Schedule, Action } from '@/lib/schedules'
import type { Locale } from '@/lib/i18n'

export interface ScheduleCardLabels extends CadenceWords {
  statusPending: string
  statusActive: string
  statusPaused: string
  statusRejected: string
  statusRetired: string
  actActivate: string
  actReject: string
  actPause: string
  actResume: string
  actRun: string
  actDelete: string
  outDone: string
  outOpen: string
  outNf: string
  nextAt: string
  noneScheduled: string
  proposedBy: string
  lastN: string
  nowArrow: string
  neverFired: string
}

export interface ScheduleCardProps {
  schedule: Schedule
  labels: ScheduleCardLabels
  lang: Locale
  busy: boolean
  onAction: (action: Action) => void
  onRun: () => void
  onDelete: () => void
  onOpenTicket: (ticket: string) => void
}

export function ScheduleCard({
  schedule: s,
  labels,
  lang,
  busy,
  onAction,
  onRun,
  onDelete,
  onOpenTicket,
}: ScheduleCardProps) {
  const terminal = s.status === 'rejected' || s.status === 'retired'
  const dimmed = s.status === 'paused' || terminal
  const occ = (s.occurrences ?? []).slice(0, 8)
  const done = occ.filter((o) => outcomeOf(o.outcome) === 'done').length
  const nf = occ.filter((o) => outcomeOf(o.outcome) === 'not_fulfilled').length

  const statusLabel =
    s.status === 'pending'
      ? labels.statusPending
      : s.status === 'active'
        ? labels.statusActive
        : s.status === 'paused'
          ? labels.statusPaused
          : s.status === 'rejected'
            ? labels.statusRejected
            : labels.statusRetired

  return (
    <Card
      size="sm"
      className={cn(
        'gap-0 px-(--card-spacing)',
        // A proposal is tinted: it is the one row that is asking something of you.
        s.status === 'pending' && 'bg-accent ring-ring',
      )}
    >
      <div className="flex flex-wrap items-center gap-2.5">
        <div>
          <div className={cn('text-[14px] font-[680]', dimmed && 'text-muted-foreground')}>
            {s.name}
          </div>
          <div className="text-muted-foreground font-mono text-[11.5px]">
            {cadenceText(s.cadence, labels)}
          </div>
        </div>
        <span className="grow" />
        <Badge
          variant={s.status === 'active' ? 'secondary' : 'outline'}
          className="rounded-[7px] px-2 py-0.5 text-[11.5px] font-[680]"
        >
          {statusLabel}
        </Badge>

        <div className="flex flex-wrap gap-1.5">
          {s.status === 'pending' && (
            <>
              <Button variant="ghost" size="sm" className="text-destructive" disabled={busy} onClick={() => onAction('reject')}>
                {labels.actReject}
              </Button>
              <Button size="sm" disabled={busy} onClick={() => onAction('activate')}>
                {labels.actActivate}
              </Button>
            </>
          )}
          {s.status === 'active' && (
            <>
              <Button variant="outline" size="sm" disabled={busy} onClick={onRun}>
                {labels.actRun}
              </Button>
              <Button variant="outline" size="sm" disabled={busy} onClick={() => onAction('pause')}>
                {labels.actPause}
              </Button>
            </>
          )}
          {s.status === 'paused' && (
            <Button size="sm" disabled={busy} onClick={() => onAction('resume')}>
              {labels.actResume}
            </Button>
          )}
          <Button variant="ghost" size="sm" className="text-destructive" disabled={busy} onClick={onDelete}>
            {labels.actDelete}
          </Button>
        </div>
      </div>

      {s.rationale && <p className="text-foreground mt-2.25 mb-0 text-[13px]">“{s.rationale}”</p>}
      {s.cadence_error && (
        <p className="text-destructive mt-2.25 mb-0 text-[13px]">{s.cadence_error}</p>
      )}

      {/* On a proposal, show the ticket it WOULD create — title and body, rendered
          the way the board renders it. Approving a cadence without seeing its
          ticket is approving a name. */}
      {s.status === 'pending' && s.template && (
        <div className="border-border bg-card mt-2.5 rounded-[9px] border border-dashed px-3 py-2.5">
          <div className="mb-1 text-[13px] font-[640]">{s.template.title}</div>
          {s.template.body && (
            <Markdown text={s.template.body} className="text-muted-foreground text-[12.5px]" />
          )}
        </div>
      )}

      <OccurrenceStrip
        occurrences={occ}
        unit={s.cadence?.every}
        lang={lang}
        labels={{
          done: labels.outDone,
          open: labels.outOpen,
          notFulfilled: labels.outNf,
          nowArrow: labels.nowArrow,
        }}
        onOpenTicket={onOpenTicket}
      />

      <div className="text-muted-foreground mt-2.5 flex flex-wrap items-center gap-2.5 font-mono text-[11.5px]">
        {occ.length ? (
          <span>
            {labels.lastN} {occ.length} —{' '}
            <b className="text-ok font-bold">
              {done} {labels.outDone}
            </b>
            {nf > 0 && (
              <>
                {' · '}
                <b className="text-nf font-bold">
                  {nf} {labels.outNf}
                </b>
              </>
            )}
          </span>
        ) : (
          <span>{labels.neverFired}</span>
        )}
        <span className="grow" />
        {s.proposed_by && (
          <span>
            {labels.proposedBy} {s.proposed_by}
          </span>
        )}
        {s.next_slot ? (
          <span>
            {labels.nextAt} {fmtWhen(s.next_slot, lang)}
          </span>
        ) : (
          s.status === 'paused' && <span>{labels.noneScheduled}</span>
        )}
      </div>
    </Card>
  )
}
