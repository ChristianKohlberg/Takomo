import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { InitiativeStatus } from '@/lib/initiatives'

// The three status tones, kept as explicit classes rather than derived: these
// are a label, not a state machine, and the colors carry meaning (parked is a
// warning tone, distilled is the success tone) that a generated palette loses.
const TONE: Record<InitiativeStatus, string> = {
  open: 'bg-secondary text-secondary-foreground',
  parked: 'bg-[rgba(201,154,58,.16)] text-[color:var(--warn,#c99a3a)]',
  distilled: 'bg-[rgba(63,122,95,.16)] text-ok',
}

export interface StatusBadgeProps {
  status: InitiativeStatus
  /** Already localized — this component does no translation. */
  label: string
  className?: string
}

export function StatusBadge({ status, label, className }: StatusBadgeProps) {
  return (
    <Badge
      variant="secondary"
      className={cn(
        'shrink-0 rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase',
        TONE[status],
        className,
      )}
    >
      {label}
    </Badge>
  )
}
