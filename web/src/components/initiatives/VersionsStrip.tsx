import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { RoadmapEpic, RoadmapLane } from '@/lib/roadmap'
import { Card } from '@/components/ui/card'

export interface VersionsStripLabels {
  /** Section heading. */
  heading: string
  /** "3 / 5 done" — receives the two numbers already formatted. */
  done: string
  ready: string
  backlog: string
  awaiting: string
  /** Shown when the lane owns no work yet. */
  empty: string
  /** How work gets filed under a lane, shown alongside `empty`. */
  emptyHint: string
  /** Warning for a parked lane whose tickets the queue still offers. */
  parkedWithReadyWork: string
}

export interface VersionsStripProps {
  /** The lane's own rollup. Absent while the roadmap is still loading. */
  lane: RoadmapLane | undefined
  /** Its versions, already resolved and ordered — see `lib/roadmap.laneVersions`. */
  versions: RoadmapEpic[]
  /** Which warnings to show, from `lib/roadmap.laneWarnings`. */
  warnings?: string[]
  /** Where a version links to. Omit and the rows render as plain text. */
  epicHref?: (id: string) => string
  labels: VersionsStripLabels
  className?: string
}

/** A 0-100 bar. Same vocabulary as the CLI's `[####------]`, in CSS. */
function Bar({ percent }: { percent: number }) {
  const pct = Math.max(0, Math.min(100, percent))
  return (
    <div
      className="bg-secondary h-1.5 w-full overflow-hidden rounded-full"
      role="progressbar"
      aria-valuenow={pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div
        className={cn('h-full rounded-full', pct === 100 ? 'bg-ok' : 'bg-accent')}
        style={{ width: `${pct}%` }}
      />
    </div>
  )
}

/**
 * The versions filed under an initiative, and where each one is.
 *
 * This is the work-shaped counterpart to `RollupStrip`, which counts what has
 * accumulated as ENTRIES. An initiative never closes, so "how far along is it"
 * is not a question its own status can answer — the answer is the state of the
 * epics filed under it, one per version.
 */
export function VersionsStrip({
  lane,
  versions,
  warnings = [],
  epicHref,
  labels,
  className,
}: VersionsStripProps) {
  if (!lane) return null

  return (
    <Card className={cn('mt-4.5 gap-0 py-0', className)}>
      <header className="border-b-border-soft flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-3.5 py-2.5">
        <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
          {labels.heading}
        </span>
        <span className="ml-auto text-[13px] tabular-nums">
          <span className="font-[720]">
            {lane.done} / {lane.total}
          </span>{' '}
          <span className="text-muted-foreground">
            {labels.done} · {lane.percent}%
          </span>
        </span>
        {warnings.includes('parked_with_ready_work') && (
          <Badge
            variant="secondary"
            className="bg-[rgba(201,154,58,.16)] text-[color:var(--warn,#c99a3a)] w-full shrink-0 rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] md:w-auto"
          >
            {labels.parkedWithReadyWork}
          </Badge>
        )}
      </header>

      {versions.length === 0 ? (
        <div className="px-3.5 py-3">
          <div className="text-[13px]">{labels.empty}</div>
          <div className="text-muted-foreground mt-0.5 text-[12px]">{labels.emptyHint}</div>
        </div>
      ) : (
        <ol className="m-0 list-none p-0">
          {versions.map((v) => {
            const href = epicHref?.(v.id)
            const title = (
              <span className="truncate text-[13.5px] font-[620]">{v.title}</span>
            )
            return (
              <li
                key={v.id}
                className="border-b-border-soft flex flex-col gap-1.5 border-b px-3.5 py-2.5 last:border-b-0"
              >
                <div className="flex items-center gap-2">
                  {href ? (
                    <a href={href} className="min-w-0 truncate hover:underline">
                      {title}
                    </a>
                  ) : (
                    <span className="min-w-0">{title}</span>
                  )}
                  <Badge
                    variant="secondary"
                    className="ml-auto shrink-0 rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase"
                  >
                    {v.state}
                  </Badge>
                  <span className="shrink-0 text-[12px] tabular-nums">
                    {v.done}/{v.total}
                  </span>
                </div>
                <Bar percent={v.percent} />
                {/* Only the non-zero counts, so a finished version does not carry
                    a row of zeroes. `awaiting` is an overlay on the others, not a
                    bucket, which is why it reads as a warning rather than a total. */}
                {(v.ready > 0 || v.backlog > 0 || v.awaiting_answer > 0) && (
                  <div className="text-muted-foreground flex flex-wrap gap-x-3 text-[11.5px] tabular-nums">
                    {v.ready > 0 && (
                      <span>
                        {labels.ready} {v.ready}
                      </span>
                    )}
                    {v.backlog > 0 && (
                      <span>
                        {labels.backlog} {v.backlog}
                      </span>
                    )}
                    {v.awaiting_answer > 0 && (
                      <span className="text-[color:var(--warn,#c99a3a)]">
                        {labels.awaiting} {v.awaiting_answer}
                      </span>
                    )}
                  </div>
                )}
              </li>
            )
          })}
        </ol>
      )}
    </Card>
  )
}
