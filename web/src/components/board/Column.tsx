// One workflow state, as a column.
//
// Columns come from the PROJECT'S workflow, not from a fixed list: a project can
// define its own states, and a board that hardcoded "todo / doing / done" would
// be wrong for every project that does.
import { useState } from 'react'
import { TicketCard } from './TicketCard'
import type { Ticket } from '@/lib/board'

/** Cards shown before the column collapses the rest behind "show more". */
const COLLAPSE_LIMIT = 6

export interface ColumnProps {
  compact?: boolean
  stateLabel?: string
  state: string
  tickets: Ticket[]
  selectedId?: string | null
  needsAnswer?: ReadonlySet<string>
  labels: { showMore: string; blocked: string; fromSchedule: string; notFulfilled: string; needsAnswer?: string }
  /** Terminal state: its cards never carry a not-fulfilled flag. */
  isDone?: boolean
  onOpen: (id: string) => void
  /** Passed to each card's schedule chip; see AppHeader.onNavigate. */
  onNavigate?: (href: string) => void
}

export function Column({
  compact,
  stateLabel,
  state,
  tickets,
  selectedId,
  needsAnswer,
  labels,
  isDone,
  onOpen,
  onNavigate,
}: ColumnProps) {
  const [expanded, setExpanded] = useState(false)
  const shown = expanded ? tickets : tickets.slice(0, COLLAPSE_LIMIT)
  const hidden = tickets.length - shown.length

  return (
    <section className="flex min-h-0 w-full shrink-0 flex-col md:w-72">
      <header className="bg-background sticky top-0 z-1 text-foreground border-border mb-3 flex items-baseline justify-between gap-2 border-b px-1 pb-3 pt-1 text-[12px] font-[650] tracking-[0.04em] uppercase">
        <span>{stateLabel ?? state.replaceAll('_', ' ')}</span>
        <span className="text-muted-foreground font-normal tabular-nums">{tickets.length}</span>
      </header>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-1 pb-2">
        {shown.map((t) => (
          <TicketCard
            key={t.id}
            compact={compact}
            ticket={t}
            selected={t.id === selectedId}
            blockedLabel={labels.blocked}
            needsAnswerLabel={needsAnswer?.has(t.id) ? labels.needsAnswer : undefined}
            scheduleLabels={{ fromSchedule: labels.fromSchedule, notFulfilled: labels.notFulfilled }}
            isDone={isDone}
            onOpen={onOpen}
            onNavigate={onNavigate}
          />
        ))}
        {hidden > 0 && (
          <button
            type="button"
            onClick={() => setExpanded(true)}
            className="text-muted-foreground hover:text-primary cursor-pointer py-1 text-[12px] font-[650]"
          >
            {labels.showMore.replace('{n}', String(hidden))}
          </button>
        )}
      </div>
    </section>
  )
}
