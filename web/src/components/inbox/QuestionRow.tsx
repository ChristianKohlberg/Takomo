import { cn } from '@/lib/utils'
import { fmtAge } from '@/lib/format'
import type { Question } from '@/lib/questions'

/** Urgency drives the left rule — the one place colour ranks work. */
const URGENCY: Record<string, string> = {
  critical: 'border-l-crit',
  high: 'border-l-high',
  normal: 'border-l-normal',
  low: 'border-l-low',
}

export interface QuestionRowProps {
  question: Question
  selected: boolean
  /** Just answered in this tab: tinted while its undo window runs. */
  landed?: boolean
  labels: { advisory: string; askedBy: string; waitingAgent: string }
  onSelect: (id: string) => void
}

export function QuestionRow({
  question: q,
  selected,
  landed,
  labels,
  onSelect,
}: QuestionRowProps) {
  return (
    <button
      type="button"
      onClick={() => onSelect(q.id)}
      aria-current={selected}
      className={cn(
        'border-b-border-soft hover:bg-muted flex w-full cursor-pointer flex-col gap-1 border-b border-l-3 px-3.5 py-3 text-left',
        URGENCY[q.urgency ?? 'normal'] ?? 'border-l-transparent',
        selected && 'bg-accent',
        landed && 'bg-okbg',
      )}
    >
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 grow truncate text-[13.5px] font-[680]">{q.title}</span>
        <span className="text-muted-foreground shrink-0 font-mono text-[11px]">
          {fmtAge(q.created_at)}
        </span>
      </div>
      <div className="text-muted-foreground flex flex-wrap items-center gap-2 font-mono text-[11px]">
        <span>{q.ticket}</span>
        {q.mode === 'advisory' && (
          <span className="bg-secondary text-secondary-foreground rounded-[5px] px-1.5">
            {labels.advisory}
          </span>
        )}
        {/* Bounced back for more research: it is not waiting on the reader. */}
        {q.awaiting === 'agent' && (
          <span className="text-warn">{labels.waitingAgent}</span>
        )}
        {q.expertise?.map((e) => (
          <span key={e} className="bg-muted rounded-[5px] px-1.5">
            {e}
          </span>
        ))}
      </div>
    </button>
  )
}
