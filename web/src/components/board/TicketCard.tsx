// Cards surface the work, its owner, and exceptions. Reference metadata lives
// in the detail panel so routine tickets stay easy to scan.
import { Card } from '@/components/ui/card'
import { cn } from '@/lib/utils'
import type { Ticket } from '@/lib/board'

export interface TicketCardProps {
  compact?: boolean
  ticket: Ticket
  selected?: boolean
  scheduleLabels?: { fromSchedule: string; notFulfilled: string }
  /** Terminal states do not get a missed-occurrence flag. */
  isDone?: boolean
  blockedLabel?: string
  /** Supplied only when an open question is waiting for a human. */
  needsAnswerLabel?: string
  onOpen: (id: string) => void
  /** Retained for existing component consumers; schedule links live in details. */
  onNavigate?: (href: string) => void
}

export function TicketCard({
  ticket: t,
  compact,
  selected,
  blockedLabel,
  needsAnswerLabel,
  scheduleLabels,
  isDone,
  onOpen,
}: TicketCardProps) {
  const blocked = (t.blocked_by?.length ?? 0) > 0 || t.state_category === 'blocked'
  const missed =
    !isDone && !!t.expires_at && new Date(t.expires_at).getTime() <= Date.now()
  const elevated = t.priority === 'high' || t.priority === 'critical'
  const hasExceptions = elevated || (blocked && blockedLabel) || needsAnswerLabel || (missed && scheduleLabels)

  return (
    <Card
      size="sm"
      className={cn(
        'relative gap-0 overflow-visible rounded-lg border border-border bg-card px-4 py-3.5 text-left shadow-sm ring-0 transition-colors hover:bg-muted/40',
        compact && 'py-2',
        selected && 'bg-accent border-ring ring-1 ring-ring',
      )}
    >
      <button
        type="button"
        onClick={() => onOpen(t.id)}
        aria-current={selected}
        aria-label={t.title || t.id}
        className="absolute inset-0 cursor-pointer rounded-lg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      />
      <div className="pointer-events-none relative">
        <div className={cn("text-[14px] leading-snug font-[650] break-words", compact && "line-clamp-2")}>{t.title || t.id}</div>
        {t.claim?.holder && (
          <div className="text-muted-foreground mt-2 text-[12px] break-words">{t.claim.holder}</div>
        )}
        {hasExceptions && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5 text-[11px] font-[650]">
            {elevated && (
              <span className={t.priority === 'critical' ? 'text-crit' : 'text-high'}>
                {t.priority}
              </span>
            )}
            {blocked && blockedLabel && (
              <span className="bg-nfbg text-nf rounded-[5px] px-1.5 py-0.5">
                {blockedLabel.replace('{n}', String(t.blocked_by?.length ?? 0))}
              </span>
            )}
            {needsAnswerLabel && (
              <span className="bg-secondary text-secondary-foreground rounded-[5px] px-1.5 py-0.5">
                {needsAnswerLabel}
              </span>
            )}
            {missed && scheduleLabels && (
              <span className="bg-nfbg text-nf rounded-[5px] px-1.5 py-0.5">
                {scheduleLabels.notFulfilled}
              </span>
            )}
          </div>
        )}
      </div>
    </Card>
  )
}
