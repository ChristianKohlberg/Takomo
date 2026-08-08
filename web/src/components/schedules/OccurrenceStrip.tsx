// The lineage strip: one cell per occurrence, most recent LAST.
//
// Right-justified rather than an 8-column grid, and that is the design decision
// rather than a layout accident: a schedule with fewer than eight occurrences
// must still put its newest cell flush right, because "now" is where the eye
// ends up. In a grid the short row left a hole on the right and the most recent
// occurrence drifted off the edge. The max-width keeps a 5-cell row the same
// cell size as an 8-cell one.
import { cn } from '@/lib/utils'
import { outcomeOf, slotLabel } from '@/lib/cadence'
import type { Occurrence, Unit } from '@/lib/schedules'
import type { Locale } from '@/lib/i18n'

export interface OccurrenceStripProps {
  /** Newest-first, as the API returns it — this component reverses for display. */
  occurrences: Occurrence[]
  unit: Unit | undefined
  lang: Locale
  labels: { done: string; open: string; notFulfilled: string; nowArrow: string }
  /** Clicking a cell opens the ticket it produced. */
  onOpenTicket: (ticket: string) => void
}

const TONE = {
  done: 'bg-okbg border-okbd',
  not_fulfilled: 'bg-nfbg border-nfbd',
  open: 'bg-accent border-ring',
} as const

const INK = {
  done: 'text-ok',
  not_fulfilled: 'text-nf',
  open: 'text-accent-foreground',
} as const

export function OccurrenceStrip({
  occurrences,
  unit,
  lang,
  labels,
  onOpenTicket,
}: OccurrenceStripProps) {
  if (!occurrences.length) return null
  const label = (o: Occurrence) => {
    const out = outcomeOf(o.outcome)
    return out === 'done' ? labels.done : out === 'not_fulfilled' ? labels.notFulfilled : labels.open
  }

  return (
    <>
      <div className="mt-2.75 flex justify-end gap-1.5">
        {occurrences
          .slice()
          .reverse()
          .map((o) => {
            const out = outcomeOf(o.outcome)
            return (
              <button
                key={o.ticket + o.slot}
                type="button"
                title={`${o.title} · ${o.ticket}`}
                onClick={() => onOpenTicket(o.ticket)}
                className={cn(
                  'hover:border-ring min-h-[62px] min-w-0 flex-[1_1_0] cursor-pointer overflow-hidden rounded-[7px] border p-2 text-left text-[11.5px] leading-[1.3]',
                  'max-w-[calc((100%-42px)/8)] max-md:max-w-[calc((100%-18px)/4)]',
                  TONE[out],
                )}
              >
                <span className="text-muted-foreground block font-mono text-[10px]">
                  {slotLabel(o.slot, unit, lang)}
                </span>
                <span className={cn('mt-0.75 block font-[660]', INK[out])}>{label(o)}</span>
                <span className="text-muted-foreground mt-0.75 block truncate font-mono text-[9.5px]">
                  {o.claimed_by || o.ticket}
                </span>
              </button>
            )
          })}
      </div>
      <div className="text-muted-foreground mt-0.75 flex justify-end font-mono text-[10px]">
        {labels.nowArrow}
      </div>
    </>
  )
}
