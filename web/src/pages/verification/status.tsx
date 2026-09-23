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
