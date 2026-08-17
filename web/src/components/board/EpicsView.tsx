import { useMemo, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { fmtAge, fmtDuration } from '@/lib/format'
import { cn } from '@/lib/utils'
import { epicAttention, STALLED_AFTER_SECONDS, type RoadmapEpic } from '@/lib/roadmap'
import {
  activeFilterCount,
  applyEpicsGrid,
  DEFAULT_FILTERS,
  DEFAULT_SORT,
  PRESET_IDS,
  type ClaimedFilter,
  type EpicsFilters,
  type EpicsSort,
  type PresetId,
  type SortKey,
} from './epicsGrid'

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
  progress: string
  /** Column headers. */
  colEpic: string
  colState: string
  colLanes: string
  colProgress: string
  colHolder: string
  colLastActivity: string
  /** Sortable header state (for aria-label). */
  sortAscending: string
  sortDescending: string
  sortNone: string
  /** Filters. */
  filters: string
  filterStateCategory: string
  filterLane: string
  filterClaimed: string
  filterAll: string
  filterClaimedYes: string
  filterClaimedNo: string
  clearFilters: string
  noMatchFilters: string
  /** Preset view buttons — labels must name the exact reading (creation vs activity). */
  presets: string
  presetRecentCreated: string
  presetNearlyComplete: string
  presetNotStarted: string
  presetStalled: string
  presetAwaiting: string
  presetUnclaimed: string
  presetFlagged: string
  unclaimed: string
  /** Last activity when the epic has no claim — honest unknown, not a dash. */
  lastActivityUnknown: string
  /** Shown beside a stalled holder line. */
  stalledMarker: string
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

const PRESET_LABEL_KEY: Record<PresetId, keyof EpicsViewLabels> = {
  recentCreated: 'presetRecentCreated',
  nearlyComplete: 'presetNearlyComplete',
  notStarted: 'presetNotStarted',
  stalled: 'presetStalled',
  awaiting: 'presetAwaiting',
  unclaimed: 'presetUnclaimed',
  flagged: 'presetFlagged',
}

function Bar({ percent }: { percent: number }) {
  const pct = Math.max(0, Math.min(100, percent))
  return (
    <div className="bg-secondary h-1.5 w-full min-w-[4rem] overflow-hidden rounded-full">
      <div
        className={cn('h-full rounded-full', pct === 100 ? 'bg-ok' : 'bg-accent')}
        style={{ width: `${pct}%` }}
      />
    </div>
  )
}

function SortableHead({
  label,
  column,
  sort,
  onSort,
  sortLabels,
}: {
  label: string
  column: SortKey
  sort: EpicsSort
  onSort: (key: SortKey) => void
  sortLabels: Pick<EpicsViewLabels, 'sortAscending' | 'sortDescending' | 'sortNone'>
}) {
  const active = sort.key === column
  const ariaSort = active
    ? sort.dir === 'asc'
      ? 'ascending'
      : 'descending'
    : 'none'
  const sortHint = active
    ? sort.dir === 'asc'
      ? sortLabels.sortAscending
      : sortLabels.sortDescending
    : sortLabels.sortNone

  return (
    <TableHead aria-sort={ariaSort}>
      <button
        type="button"
        onClick={() => onSort(column)}
        className="hover:text-foreground flex w-full cursor-pointer items-center gap-1 text-left"
        aria-label={`${label}, ${sortHint}`}
      >
        <span>{label}</span>
        <span className="text-[10px] tabular-nums opacity-70" aria-hidden>
          {active ? (sort.dir === 'asc' ? '↑' : '↓') : '↕'}
        </span>
      </button>
    </TableHead>
  )
}

/**
 * The project at epic altitude: a scannable grid with sortable columns, filters,
 * and preset views onto common questions.
 *
 * This is not the board grouped by epic — that is still a ticket board, and
 * answers "where is each ticket". This answers "where is each epic, who has it,
 * and is it moving", which became a question worth a surface when an epic claim
 * started reserving its whole subtree with no expiry: a held epic that nothing
 * is happening under is invisible on a ticket board, and no lease will lapse and
 * hand it back.
 *
 * Rows start in server creation order, but the grid can sort and filter client-
 * side — that is presentation, not inventing API priority. The attention strip
 * still counts every epic before you scroll. "Recently created" is the reverse
 * of server order; last-activity is only knowable for claimed epics in today's
 * payload, so unclaimed rows say unknown rather than implying zero movement.
 */
export function EpicsView({
  epics,
  laneTitles,
  onOpen,
  stalledAfter = STALLED_AFTER_SECONDS,
  labels,
  className,
}: EpicsViewProps) {
  const [filters, setFilters] = useState<EpicsFilters>(DEFAULT_FILTERS)
  const [sort, setSort] = useState<EpicsSort>(DEFAULT_SORT)
  const [preset, setPreset] = useState<PresetId | null>(null)

  const stateCategories = useMemo(() => {
    const set = new Set<string>()
    for (const e of epics) {
      if (e.state_category) set.add(e.state_category)
    }
    return [...set].sort()
  }, [epics])

  const laneOptions = useMemo(() => {
    return Object.entries(laneTitles)
      .map(([id, title]) => ({ id, title }))
      .sort((a, b) => a.title.localeCompare(b.title))
  }, [laneTitles])

  const visible = useMemo(
    () => applyEpicsGrid(epics, filters, sort, preset, stalledAfter),
    [epics, filters, sort, preset, stalledAfter],
  )

  const filterCount = activeFilterCount(filters)

  const onSort = (key: SortKey) => {
    setPreset(null)
    setSort((prev) =>
      prev.key === key
        ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' }
        : { key, dir: key === 'title' || key === 'state' ? 'asc' : 'desc' },
    )
  }

  const onPreset = (id: PresetId) => {
    setPreset((cur) => (cur === id ? null : id))
  }

  const clearFilters = () => {
    setFilters(DEFAULT_FILTERS)
    setPreset(null)
  }

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

      <div className="flex flex-col gap-2">
        <div className="text-muted-foreground text-[11px] font-[750] tracking-[0.06em] uppercase">
          {labels.presets}
        </div>
        <div className="flex flex-wrap gap-1.5" role="group" aria-label={labels.presets}>
          {PRESET_IDS.map((id) => (
            <Button
              key={id}
              type="button"
              size="sm"
              variant={preset === id ? 'default' : 'secondary'}
              className="h-7 rounded-[6px] px-2.5 text-[12px] font-[650]"
              onClick={() => onPreset(id)}
              aria-pressed={preset === id}
            >
              {labels[PRESET_LABEL_KEY[id]]}
            </Button>
          ))}
        </div>
      </div>

      <div
        className="bg-card border-border flex flex-col gap-3 rounded-[10px] border px-3 py-2.5 md:flex-row md:flex-wrap md:items-end"
        aria-label={labels.filters}
      >
        <div className="flex min-w-0 flex-col gap-1">
          <Label htmlFor="epics-filter-category" className="text-[11px] font-[650]">
            {labels.filterStateCategory}
          </Label>
          <select
            id="epics-filter-category"
            value={filters.stateCategory}
            onChange={(e) => {
              setPreset(null)
              setFilters((f) => ({ ...f, stateCategory: e.target.value }))
            }}
            className="border-border bg-background h-8 min-w-0 rounded-[6px] border px-2 text-[13px] md:min-w-[9rem]"
          >
            <option value="">{labels.filterAll}</option>
            {stateCategories.map((c) => (
              <option key={c} value={c}>{c}</option>
            ))}
          </select>
        </div>
        <div className="flex min-w-0 flex-col gap-1">
          <Label htmlFor="epics-filter-lane" className="text-[11px] font-[650]">
            {labels.filterLane}
          </Label>
          <select
            id="epics-filter-lane"
            value={filters.lane}
            onChange={(e) => {
              setPreset(null)
              setFilters((f) => ({ ...f, lane: e.target.value }))
            }}
            className="border-border bg-background h-8 min-w-0 rounded-[6px] border px-2 text-[13px] md:min-w-[9rem]"
          >
            <option value="">{labels.filterAll}</option>
            {laneOptions.map((l) => (
              <option key={l.id} value={l.id}>{l.title}</option>
            ))}
          </select>
        </div>
        <div className="flex min-w-0 flex-col gap-1">
          <Label htmlFor="epics-filter-claimed" className="text-[11px] font-[650]">
            {labels.filterClaimed}
          </Label>
          <select
            id="epics-filter-claimed"
            value={filters.claimed}
            onChange={(e) => {
              setPreset(null)
              setFilters((f) => ({ ...f, claimed: e.target.value as ClaimedFilter }))
            }}
            className="border-border bg-background h-8 min-w-0 rounded-[6px] border px-2 text-[13px] md:min-w-[9rem]"
          >
            <option value="all">{labels.filterAll}</option>
            <option value="claimed">{labels.filterClaimedYes}</option>
            <option value="unclaimed">{labels.filterClaimedNo}</option>
          </select>
        </div>
        {(filterCount > 0 || preset) && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 text-[12px] font-[650] underline"
            onClick={clearFilters}
          >
            {labels.clearFilters}
            {filterCount > 0 ? ` (${filterCount})` : ''}
          </Button>
        )}
      </div>

      {visible.length === 0 ? (
        <div className="text-muted-foreground px-2 py-10 text-center">
          <div className="text-[13.5px]">{labels.noMatchFilters}</div>
          <button
            type="button"
            onClick={clearFilters}
            className="text-primary mt-2 cursor-pointer px-2 py-2 text-[13px] font-[650] underline"
          >
            {labels.clearFilters}
          </button>
        </div>
      ) : (
        <div className="bg-card border-border overflow-x-auto rounded-[10px] border">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <SortableHead
                  label={labels.colEpic}
                  column="title"
                  sort={sort}
                  onSort={onSort}
                  sortLabels={labels}
                />
                <SortableHead
                  label={labels.colState}
                  column="state"
                  sort={sort}
                  onSort={onSort}
                  sortLabels={labels}
                />
                <TableHead>{labels.colLanes}</TableHead>
                <SortableHead
                  label={labels.colProgress}
                  column="progress"
                  sort={sort}
                  onSort={onSort}
                  sortLabels={labels}
                />
                <SortableHead
                  label={labels.colHolder}
                  column="holder"
                  sort={sort}
                  onSort={onSort}
                  sortLabels={labels}
                />
                <SortableHead
                  label={labels.colLastActivity}
                  column="lastActivity"
                  sort={sort}
                  onSort={onSort}
                  sortLabels={labels}
                />
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.map((e) => {
                const lanes = (e.initiatives ?? []).map((id) => laneTitles[id] ?? id)
                const idle = e.claim?.idle_seconds ?? null
                const stalled = e.claim != null && idle != null && idle >= stalledAfter
                return (
                  <TableRow
                    key={e.id}
                    className={cn(stalled && 'border-[color:var(--warn,#c99a3a)]')}
                  >
                    <TableCell className="min-w-[10rem]">
                      <button
                        type="button"
                        onClick={() => onOpen(e.id)}
                        className="hover:text-primary block max-w-full cursor-pointer text-left"
                      >
                        <span className="text-[13.5px] font-[650]">{e.title}</span>
                        <span className="text-muted-foreground mt-0.5 block text-[11px] tabular-nums">
                          {e.id}
                        </span>
                        {e.flags.length > 0 && (
                          <div className="mt-1.5 flex flex-wrap gap-1">
                            {e.flags.map((f) => (
                              <Badge
                                key={f}
                                variant="secondary"
                                className="rounded-[5px] px-1.5 py-0 text-[10px] font-[700]"
                              >
                                {f}
                              </Badge>
                            ))}
                          </div>
                        )}
                      </button>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant="secondary"
                        className="rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase"
                      >
                        {e.state}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground max-w-[12rem] text-[12px]">
                      {lanes.length > 0 ? lanes.join(' · ') : labels.noLane}
                    </TableCell>
                    <TableCell className="min-w-[8rem]">
                      <div className="flex flex-col gap-1">
                        <Bar percent={e.percent} />
                        <span
                          className="text-[12px] tabular-nums"
                          aria-label={labels.progress}
                        >
                          {e.done}/{e.total} · {e.percent}%
                        </span>
                        {(e.ready > 0 || e.backlog > 0 || e.awaiting_answer > 0) && (
                          <span className="text-muted-foreground text-[11px] tabular-nums">
                            {e.ready > 0 && `${labels.ready} ${e.ready}`}
                            {e.ready > 0 && e.backlog > 0 && ' · '}
                            {e.backlog > 0 && `${labels.backlog} ${e.backlog}`}
                            {(e.ready > 0 || e.backlog > 0) && e.awaiting_answer > 0 && ' · '}
                            {e.awaiting_answer > 0 && (
                              <span className="text-[color:var(--warn,#c99a3a)]">
                                {labels.awaiting} {e.awaiting_answer}
                              </span>
                            )}
                          </span>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="min-w-[9rem] text-[12px]">
                      {e.claim ? (
                        <span className={cn(stalled && 'text-[color:var(--warn,#c99a3a)]')}>
                          {labels.heldBy} {e.claim.holder}
                          <span className="text-muted-foreground block text-[11px]">
                            {e.claim.indefinite
                              ? labels.indefinite
                              : fmtDuration(e.claim.held_for_seconds)}
                            {' · '}
                            {labels.idle} {fmtDuration(idle)}
                            {stalled && (
                              <>
                                {' · '}
                                <span className="text-[color:var(--warn,#c99a3a)]">
                                  {labels.stalledMarker}
                                </span>
                              </>
                            )}
                          </span>
                        </span>
                      ) : (
                        <span className="text-muted-foreground">{labels.unclaimed}</span>
                      )}
                    </TableCell>
                    <TableCell className="text-[12px] tabular-nums">
                      {e.claim?.last_activity_at
                        ? fmtAge(e.claim.last_activity_at)
                        : (
                          <span className="text-muted-foreground">{labels.lastActivityUnknown}</span>
                        )}
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  )
}
