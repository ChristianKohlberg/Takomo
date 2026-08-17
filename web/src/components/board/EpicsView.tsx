import { Badge } from '@/components/ui/badge'
import { fmtDuration } from '@/lib/format'
import { cn } from '@/lib/utils'
import { epicAttention, STALLED_AFTER_SECONDS, type RoadmapEpic } from '@/lib/roadmap'

export interface EpicsViewLabels {
  /** "2 held · 1 stalled · …" cells. */
  held: string
  stalled: string
  awaiting: string
  flagged: string
  /** Row detail. */
  ready: string
  backlog: string
  heldBy: string
  idle: string
  indefinite: string
  noLane: string
  /** Shown when the project has no epics at all. */
  empty: string
  emptyHint: string
  /** Column-ish headings for the row body. */
  progress: string
}

export interface EpicsViewProps {
  epics: RoadmapEpic[]
  /** Lane id → title, from `lib/roadmap.laneTitles`. */
  laneTitles: Record<string, string>
  onOpen: (id: string) => void
  /** Seconds of no movement past which a held epic is called stalled. */
  stalledAfter?: number
  labels: EpicsViewLabels
  className?: string
}

function Bar({ percent }: { percent: number }) {
  const pct = Math.max(0, Math.min(100, percent))
  return (
    <div className="bg-secondary h-1.5 w-full overflow-hidden rounded-full">
      <div
        className={cn('h-full rounded-full', pct === 100 ? 'bg-ok' : 'bg-accent')}
        style={{ width: `${pct}%` }}
      />
    </div>
  )
}

/**
 * The project at epic altitude: one row per epic, no columns.
 *
 * This is not the board grouped by epic — that is still a ticket board, and
 * answers "where is each ticket". This answers "where is each epic, who has it,
 * and is it moving", which became a question worth a surface when an epic claim
 * started reserving its whole subtree with no expiry: a held epic that nothing
 * is happening under is invisible on a ticket board, and no lease will lapse and
 * hand it back.
 *
 * Rows stay in the server's order — creation order — rather than being ranked
 * here. A ranking would be this component inventing a priority the API does not
 * have; the attention strip gives the fast read instead.
 */
export function EpicsView({
  epics,
  laneTitles,
  onOpen,
  stalledAfter = STALLED_AFTER_SECONDS,
  labels,
  className,
}: EpicsViewProps) {
  if (epics.length === 0) {
    return (
      <div className="text-muted-foreground px-2 py-14 text-center">
        <div className="text-foreground text-[13.5px]">{labels.empty}</div>
        <div className="mt-1 text-[12.5px]">{labels.emptyHint}</div>
      </div>
    )
  }

  const attention = epicAttention(epics, stalledAfter)
  const cells: [string, number, boolean][] = [
    [labels.held, attention.held, false],
    [labels.stalled, attention.stalled, attention.stalled > 0],
    [labels.awaiting, attention.awaiting, attention.awaiting > 0],
    [labels.flagged, attention.flagged, attention.flagged > 0],
  ]

  return (
    <div className={cn('flex min-h-0 flex-col gap-3 overflow-y-auto', className)}>
      {/* The answer before the scroll: what wants a person, counted once. */}
      <div className="bg-card border-border flex flex-wrap gap-x-4 gap-y-1 rounded-[10px] border px-3.5 py-2.5">
        {cells.map(([label, n, warn]) => (
          <span key={label} className="text-[12.5px] tabular-nums">
            <span className={cn('font-[720]', warn && 'text-[color:var(--warn,#c99a3a)]')}>{n}</span>{' '}
            <span className="text-muted-foreground">{label}</span>
          </span>
        ))}
      </div>

      <ul className="m-0 flex list-none flex-col gap-2 p-0">
        {epics.map((e) => {
          const lanes = (e.initiatives ?? []).map((id) => laneTitles[id] ?? id)
          const idle = e.claim?.idle_seconds ?? null
          const stalled = e.claim != null && idle != null && idle >= stalledAfter
          return (
            <li key={e.id}>
              <button
                type="button"
                onClick={() => onOpen(e.id)}
                className={cn(
                  'bg-card border-border hover:border-ring block w-full cursor-pointer rounded-[10px] border px-3.5 py-3 text-left',
                  stalled && 'border-[color:var(--warn,#c99a3a)]',
                )}
              >
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
                  <span className="text-[14px] font-[650]">{e.title}</span>
                  <span className="text-muted-foreground text-[11.5px] tabular-nums">{e.id}</span>
                  <Badge
                    variant="secondary"
                    className="ml-auto shrink-0 rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase"
                  >
                    {e.state}
                  </Badge>
                </div>

                {/* Which lanes this version belongs to — the relation a ticket
                    board cannot show, because a lane is not a ticket. */}
                <div className="text-muted-foreground mt-1 text-[12px]">
                  {lanes.length > 0 ? lanes.join(' · ') : labels.noLane}
                </div>

                <div className="mt-2 flex items-center gap-2">
                  <div className="min-w-0 flex-1">
                    <Bar percent={e.percent} />
                  </div>
                  <span
                    className="shrink-0 text-[12px] tabular-nums"
                    aria-label={labels.progress}
                  >
                    {e.done}/{e.total} · {e.percent}%
                  </span>
                </div>

                {(e.ready > 0 || e.backlog > 0 || e.awaiting_answer > 0 || e.claim) && (
                  <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-0.5 text-[11.5px] tabular-nums">
                    {e.ready > 0 && (
                      <span className="text-muted-foreground">
                        {labels.ready} {e.ready}
                      </span>
                    )}
                    {e.backlog > 0 && (
                      <span className="text-muted-foreground">
                        {labels.backlog} {e.backlog}
                      </span>
                    )}
                    {e.awaiting_answer > 0 && (
                      <span className="text-[color:var(--warn,#c99a3a)]">
                        {labels.awaiting} {e.awaiting_answer}
                      </span>
                    )}
                    {e.claim && (
                      <span className={cn(stalled && 'text-[color:var(--warn,#c99a3a)]')}>
                        {labels.heldBy} {e.claim.holder} ·{' '}
                        {e.claim.indefinite
                          ? labels.indefinite
                          : fmtDuration(e.claim.held_for_seconds)}{' '}
                        · {labels.idle} {fmtDuration(idle)}
                      </span>
                    )}
                  </div>
                )}

                {e.flags.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1.5">
                    {e.flags.map((f) => (
                      <Badge
                        key={f}
                        variant="secondary"
                        className="rounded-[5px] px-1.75 py-0.5 text-[10px] font-[700]"
                      >
                        {f}
                      </Badge>
                    ))}
                  </div>
                )}
              </button>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
