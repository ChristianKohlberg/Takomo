import { CircleCheck, CircleDashed, CircleX, Clock } from 'lucide-react'
import type { BehaviorStatus } from '@/lib/behaviors'
import { cn } from '@/lib/utils'

export type StatusLabels = Record<
  'statusVerified' | 'statusFailing' | 'statusStale' | 'statusUntested',
  string
>

const tone: Record<BehaviorStatus, string> = {
  verified: 'border-okbd bg-okbg text-ok',
  failing: 'border-nfbd bg-nfbg text-nf',
  stale: 'border-warn/40 text-warn',
  untested: 'bg-muted text-muted-foreground',
}

export function statusLabel(status: BehaviorStatus, t: StatusLabels): string {
  switch (status) {
    case 'verified':
      return t.statusVerified
    case 'failing':
      return t.statusFailing
    case 'stale':
      return t.statusStale
    case 'untested':
      return t.statusUntested
  }
}

export function StatusBadge({ status, t }: { status: BehaviorStatus; t: StatusLabels }) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center rounded-md border border-transparent px-2 py-0.5 text-xs font-medium',
        tone[status],
      )}
    >
      {statusLabel(status, t)}
    </span>
  )
}

export function OutcomeMark({ outcome, label }: { outcome: 'pass' | 'fail'; label: string }) {
  return (
    <span className={cn('text-xs font-semibold', outcome === 'pass' ? 'text-ok' : 'text-nf')}>
      {label}
    </span>
  )
}

/** Most urgent first: what needs doing before what is done. */
export const STATUS_ORDER: Record<BehaviorStatus, number> = { failing: 0, stale: 1, untested: 2, verified: 3 }

export const statusText: Record<BehaviorStatus, string> = {
  verified: 'text-ok',
  failing: 'text-nf',
  stale: 'text-warn',
  untested: 'text-muted-foreground',
}

/** Fill for a progress segment of this status. */
export const statusFill: Record<BehaviorStatus, string> = {
  verified: 'bg-ok',
  failing: 'bg-nf',
  stale: 'bg-warn',
  untested: 'bg-muted-foreground/25',
}

const icons = { verified: CircleCheck, failing: CircleX, stale: Clock, untested: CircleDashed }

/** The status as a shape as well as a colour, so it reads without colour too. */
export function StatusIcon({ status, label, className }: { status: BehaviorStatus; label?: string; className?: string }) {
  const Icon = icons[status]
  return (
    <Icon
      className={cn('size-4 shrink-0', statusText[status], className)}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? 'img' : undefined}
    />
  )
}

/** One bar, one segment per status in urgency order; empty when there is nothing to count. */
export function StatusBar({
  counts,
  label,
  className,
}: {
  counts: Record<BehaviorStatus, number> & { total: number }
  label: string
  className?: string
}) {
  const order: BehaviorStatus[] = ['verified', 'failing', 'stale', 'untested']
  return (
    <div
      role="img"
      aria-label={label}
      className={cn('bg-muted flex h-2 w-full overflow-hidden rounded-full', className)}
    >
      {counts.total > 0 &&
        order.map((status) =>
          counts[status] > 0 ? (
            <span key={status} className={statusFill[status]} style={{ width: `${(counts[status] / counts.total) * 100}%` }} />
          ) : null,
        )}
    </div>
  )
}
