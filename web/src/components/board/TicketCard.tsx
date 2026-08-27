// One ticket, as a card.
//
// Each attribute is encoded ONCE: priority is a single coloured word, not a
// stripe and a word and a dot; identifiers are monospace because they are
// identifiers; and a claimed ticket says who holds it rather than adding a
// badge whose colour you would have to learn.
import { Card } from '@/components/ui/card'
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
  /** Client-side navigation for the schedule chip; see AppHeader.onNavigate. */
  onNavigate?: (href: string) => void
}

export function TicketCard({
  ticket: t,
  selected,
  blockedLabel,
  scheduleLabels,
  isDone,
  onOpen,
  onNavigate,
}: TicketCardProps) {
  const blocked = (t.blocked_by?.length ?? 0) > 0
  // An occurrence whose deadline passed has stopped counting as live work, and
  // the server transitions NOTHING when that happens — so this card is the only
  // place a reader learns it.
  const notFulfilled =
    !isDone && !!t.expires_at && new Date(t.expires_at).getTime() <= Date.now()
  return (
    // A stretched button rather than a button wrapping everything.
    //
    // The schedule chip is a real link to /schedules, and an <a> inside a
    // <button> is invalid HTML — which is what it used to be, as a
    // `span[role="link"]` with no href at all, so middle-click, cmd-click and
    // "copy link address" silently did nothing. Here the button covers the card
    // for the open-ticket action, the content sits above it with
    // `pointer-events-none` so clicks fall through, and the chip re-enables
    // pointer events for itself. Two real, unnested interactive elements.
    <Card
      size="sm"
      className={cn(
        'hover:ring-ring relative gap-0 px-(--card-spacing) text-left',
        // `overflow-visible` undoes Card's own `overflow-hidden`, and it is not
        // cosmetic. The open-ticket button below is `absolute inset-0` — it fills
        // the card's padding box exactly — and it has no focus class of its own,
        // so its focus indicator is the UA outline that `outline-ring/50` in
        // globals.css colours. An outline is painted OUTSIDE the border box, and
        // `overflow: hidden` clips at the padding box, so with Card's default
        // every ticket on /board would still take keyboard focus while showing
        // nothing for it. Nothing here needs the clip: the card has no image
        // children and a background is clipped by border-radius regardless.
        'overflow-visible',
        selected && 'bg-accent ring-ring',
      )}
    >
      <button
        type="button"
        onClick={() => onOpen(t.id)}
        aria-current={selected}
        aria-label={t.title || t.id}
        className="absolute inset-0 cursor-pointer rounded-xl"
      />
      <div className="pointer-events-none relative">
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
        <a
          href="/schedules"
          title={`${scheduleLabels.fromSchedule}: ${t.schedule}`}
          onClick={(e) => {
            e.stopPropagation()
            // Same rule as the header nav: only a plain left-click is
            // intercepted, so cmd-click still opens a new tab.
            if (!onNavigate) return
            if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
            e.preventDefault()
            onNavigate('/schedules')
          }}
          className="text-muted-foreground pointer-events-auto mt-1.5 -mx-1 -my-0.5 flex w-fit cursor-pointer items-center gap-1 px-1 py-0.5 font-mono text-[10.5px] no-underline"
        >
          <span>{'\u21bb'}</span>
          <span>{t.schedule}</span>
        </a>
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
      </div>
    </Card>
  )
}
