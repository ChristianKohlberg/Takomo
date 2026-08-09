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
  state: string
  tickets: Ticket[]
  selectedId?: string | null
  labels: { showMore: string; blocked: string; fromSchedule: string; notFulfilled: string }
  /** Terminal state: its cards never carry a not-fulfilled flag. */
  isDone?: boolean
  onOpen: (id: string) => void
  /** Passed to each card's schedule chip; see AppHeader.onNavigate. */
  onNavigate?: (href: string) => void
}

export function Column({
  state,
  tickets,
  selectedId,
  labels,
  isDone,
  onOpen,
  onNavigate,
}: ColumnProps) {
  const [expanded, setExpanded] = useState(false)
  const shown = expanded ? tickets : tickets.slice(0, COLLAPSE_LIMIT)
  const hidden = tickets.length - shown.length

  return (
    <section className="bg-muted/40 flex min-h-0 w-full shrink-0 flex-col rounded-[10px] md:w-72">
      <header className="text-muted-foreground flex items-baseline gap-2 px-3 py-2 text-[11.5px] font-[750] tracking-[0.05em] uppercase">
        <span>{state}</span>
        <span className="font-semibold tabular-nums">{tickets.length}</span>
      </header>
      <div className="flex min-h-0 flex-col gap-2 overflow-y-auto px-2 pb-2">
        {shown.map((t) => (
          <TicketCard
            key={t.id}
            ticket={t}
            selected={t.id === selectedId}
            blockedLabel={labels.blocked}
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
