// One ticket, as a card.
//
// Each attribute is encoded ONCE: priority is a single coloured word, not a
// stripe and a word and a dot; identifiers are monospace because they are
// identifiers; and a claimed ticket says who holds it rather than adding a
// badge whose colour you would have to learn.
import { cn } from '@/lib/utils'
import { fmtAge } from '@/lib/format'
import type { Ticket } from '@/lib/board'

const PRIORITY: Record<string, string> = {
  critical: 'text-crit',
  high: 'text-high',
  normal: 'text-normal',
  low: 'text-low',
}

const LEVEL: Record<string, number> = { critical: 4, high: 3, normal: 2, low: 1 }
const BAR: Record<string, string> = {
  critical: 'bg-primary',
  high: 'bg-high',
  normal: 'bg-normal',
  low: 'bg-low',
}

/**
 * Urgency as four bars. It repeats the coloured word rather than replacing it —
 * the word is what a screen reader gets, the gauge is what the eye scans down a
 * column of forty cards.
 */
function Gauge({ priority }: { priority: string }) {
  const lvl = LEVEL[priority] ?? 1
  return (
    <span aria-hidden className="inline-flex items-end gap-[2px]">
      {[1, 2, 3, 4].map((i) => (
        <span
          key={i}
          className={cn('w-[3px] rounded-[1px]', i <= lvl ? (BAR[priority] ?? 'bg-low') : 'bg-gaugeoff')}
          style={{ height: 3 + i * 2 }}
        />
      ))}
    </span>
  )
}

export interface TicketCardProps {
  ticket: Ticket
  selected?: boolean
  /** `fromSchedule` and `notFulfilled` — see where they are used below. */
  scheduleLabels?: { fromSchedule: string; notFulfilled: string }
  /** Terminal states do not get a not-fulfilled flag: finished is finished. */
  isDone?: boolean
  /**
   * Template for the blocked chip, with `{n}` for the count. The count comes
   * from the ticket's OWN dependencies — a card knows what blocks it; it does
   * not know about questions, which is the drawer's callout.
   */
  blockedLabel?: string
  onOpen: (id: string) => void
}

export function TicketCard({
  ticket: t,
  selected,
  blockedLabel,
  scheduleLabels,
  isDone,
  onOpen,
}: TicketCardProps) {
  const blocked = (t.blocked_by?.length ?? 0) > 0
  // An occurrence whose deadline passed has stopped counting as live work, and
  // the server transitions NOTHING when that happens — so this card is the only
  // place a reader learns it.
  const notFulfilled =
    !isDone && !!t.expires_at && new Date(t.expires_at).getTime() <= Date.now()
  return (
    <button
      type="button"
      onClick={() => onOpen(t.id)}
      aria-current={selected}
      className={cn(
        'bg-card border-border hover:border-ring w-full cursor-pointer rounded-[9px] border px-3 py-2.5 text-left',
        selected && 'bg-accent border-ring',
      )}
    >
      <div className="flex items-baseline gap-2">
        <span className="text-muted-foreground shrink-0 font-mono text-[11px]">{t.id}</span>
        {t.priority && (
          <>
            <Gauge priority={t.priority} />
            <span className={cn('text-[11px] font-[680]', PRIORITY[t.priority] ?? 'text-normal')}>
              {t.priority}
            </span>
          </>
        )}
        <span className="grow" />
        {notFulfilled && scheduleLabels && (
          <span className="bg-nfbg text-nf shrink-0 rounded-[5px] px-1.5 text-[10.5px] font-[680]">
            {scheduleLabels.notFulfilled}
          </span>
        )}
        <span className="text-muted-foreground shrink-0 font-mono text-[11px]">
          {fmtAge(t.updated_at ?? t.created_at)}
        </span>
      </div>

      <div className="mt-1 text-[13.2px] font-[650] break-words">{t.title}</div>

      {/* Where a scheduled ticket came from. It links to /schedules rather than
          opening the ticket, so the two pages stay one product. */}
      {t.schedule && scheduleLabels && (
        <span
          title={`${scheduleLabels.fromSchedule}: ${t.schedule}`}
          role="link"
          tabIndex={0}
          onClick={(e) => {
            e.stopPropagation()
            window.location.href = '/schedules'
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.stopPropagation()
              window.location.href = '/schedules'
            }
          }}
          className="text-muted-foreground mt-1.5 flex w-fit cursor-pointer items-center gap-1 font-mono text-[10.5px]"
        >
          <span>{'\u21bb'}</span>
          <span>{t.schedule}</span>
        </span>
      )}

      {(t.labels?.length || t.tags?.length || t.claim?.holder || blocked) && (
        <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px]">
          {blocked && blockedLabel && (
            <span className="bg-nfbg text-nf rounded-[5px] px-1.5 font-[650]">
              {blockedLabel.replace('{n}', String(t.blocked_by?.length ?? 0))}
            </span>
          )}
          {t.claim?.holder && (
            <span className="text-muted-foreground font-mono">⚑ {t.claim.holder}</span>
          )}
          {t.labels?.map((l) => (
            <span key={l} className="bg-muted border-border rounded-[5px] border px-1.5">
              {l}
            </span>
          ))}
          {t.tags?.map((tag) => (
            <span
              key={tag}
              className="bg-secondary text-secondary-foreground rounded-[5px] px-1.5 font-mono"
            >
              {tag}
            </span>
          ))}
        </div>
      )}
    </button>
  )
}
