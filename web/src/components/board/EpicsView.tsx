import { useMemo, useState } from 'react'
import { AlertTriangle, Ban, CheckCircle2, Circle, Clock3, LoaderCircle, MessageCircle, Plus, Search, SlidersHorizontal } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Field } from '@/components/Field'
import { Picker } from '@/components/Picker'
import { fmtAge, fmtDuration } from '@/lib/format'
import { cn } from '@/lib/utils'
import { STALLED_AFTER_SECONDS, type RoadmapEpic } from '@/lib/roadmap'
import {
  activeFilterCount, activityAt, applyEpicsGrid, DEFAULT_FILTERS, DEFAULT_SORT,
  epicWarnings, isStalled, type ClaimedFilter, type EpicsFilters, type EpicsSort,
} from './epicsGrid'
import type { EpicsViewLabels } from './epics-strings'
export type { EpicsViewLabels } from './epics-strings'

export interface EpicsViewProps {
  epics: RoadmapEpic[]
  laneTitles: Record<string, string>
  onOpen: (id: string) => void
  onCreate?: () => void
  canCreate?: boolean
  terminalStates?: string[]
  stalledAfter?: number
  labels: EpicsViewLabels
  className?: string
}
const sentence = (s: string) => s.replace(/[_-]/g, ' ').replace(/^./, (c) => c.toUpperCase())
const fill = (s: string, values: Record<string, string | number>) => s.replace(/\{(\w+)\}/g, (all, key: string) => String(values[key] ?? all))
const columns = 'lg:grid-cols-[minmax(0,1fr)_minmax(0,8rem)_minmax(0,8rem)_minmax(0,12rem)]'

function State({ epic }: { epic: RoadmapEpic }) {
  const Icon = epic.state_category === 'done' ? CheckCircle2
    : epic.state_category === 'cancelled' ? Ban
    : epic.state_category === 'blocked' ? AlertTriangle
    : epic.state_category === 'in_progress' || epic.state_category === 'review' ? LoaderCircle : Circle
  return <span className={cn('flex min-w-0 items-center gap-1.5 text-[12px]',
    epic.state_category === 'blocked' ? 'text-[color:var(--warn)]' : 'text-muted-foreground')}>
    <Icon size={14} className="shrink-0" aria-hidden />
    <span className="truncate">{sentence(epic.state)}</span>
  </span>
}

export function EpicsView({ epics, laneTitles, onOpen, onCreate, canCreate = false,
  terminalStates, stalledAfter = STALLED_AFTER_SECONDS, labels: t, className }: EpicsViewProps) {
  const [filters, setFilters] = useState<EpicsFilters>(DEFAULT_FILTERS)
  const [sort, setSort] = useState<EpicsSort>(DEFAULT_SORT)
  const visible = useMemo(() => applyEpicsGrid(epics, filters, sort, stalledAfter, terminalStates),
    [epics, filters, sort, stalledAfter, terminalStates])
  const states = [...new Set(epics.map((e) => e.state))].sort()
  const lanes = [...new Set(epics.flatMap((e) => e.initiatives ?? []))]
    .map((id) => ({ value: id, label: laneTitles[id] ?? id })).sort((a, b) => a.label.localeCompare(b.label))
  const count = activeFilterCount(filters)
  const summary = [filters.state && sentence(filters.state), filters.lane && (laneTitles[filters.lane] ?? filters.lane),
    filters.claimed !== 'all' && (filters.claimed === 'claimed' ? t.claimed : t.unclaimed)].filter(Boolean).join(' · ')
  const clear = () => setFilters(DEFAULT_FILTERS)
  const update = (patch: Partial<EpicsFilters>) => setFilters((prev) => ({ ...prev, ...patch }))

  return <section className={cn('flex min-h-0 min-w-0 flex-col', className)} aria-label={t.title}>
    <div className="mb-3 flex flex-none items-center gap-2.5">
      <h1 className="text-lg font-semibold tracking-tight">{t.title}</h1>
      <span className="text-muted-foreground text-xs tabular-nums" role="status">
        {fill(t.count, { visible: visible.length, total: epics.length })}
      </span>
      {onCreate && canCreate && <Button size="sm" className="ml-auto" onClick={onCreate}>
        <Plus size={15} aria-hidden />{t.newEpic}
      </Button>}
    </div>
    <div className="mb-3 flex flex-none flex-wrap items-center gap-2" aria-label={t.filters}>
      <div className="relative min-w-0 flex-[1_1_12rem] lg:max-w-72">
        <Search size={14} className="text-muted-foreground pointer-events-none absolute top-2 left-2.5" aria-hidden />
        <Input type="search" aria-label={t.search} placeholder={t.search} className="h-8 pl-8"
          value={filters.search} onChange={(e) => update({ search: e.target.value })} />
      </div>
      <div role="group" aria-label={t.visibility} className="bg-muted flex shrink-0 gap-0.5 rounded-lg p-0.5">
        {(['active', 'all'] as const).map((scope) => <button key={scope} type="button"
          aria-pressed={filters.scope === scope} onClick={() => update({ scope })}
          className={cn('cursor-pointer rounded-md px-2.5 py-1 text-xs focus-visible:outline-2 focus-visible:outline-offset-2',
            filters.scope === scope ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground')}>
          {t[scope]}
        </button>)}
      </div>
      <Popover>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm" className="h-8" aria-label={count ? `${t.filters} (${count})` : t.filters}>
            <SlidersHorizontal size={14} aria-hidden />{t.filters}
            {count > 0 && <span className="bg-secondary rounded px-1 text-[11px] tabular-nums">{count}</span>}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" aria-label={t.filters} className="max-w-[calc(100vw-2rem)] gap-3 p-3">
          <Field label={t.state}>{(id) => <Picker id={id} value={filters.state}
            onValueChange={(state) => update({ state })}
            options={[{ value: '', label: t.anyState }, ...states.map((state) => ({ value: state, label: sentence(state) }))]} />}</Field>
          <Field label={t.initiative}>{(id) => <Picker id={id} value={filters.lane}
            onValueChange={(lane) => update({ lane })} options={[{ value: '', label: t.anyInitiative }, ...lanes]} />}</Field>
          <Field label={t.claim}>{(id) => <Picker id={id} value={filters.claimed}
            onValueChange={(claimed) => update({ claimed: claimed as ClaimedFilter })}
            options={[{ value: 'all', label: t.anyClaim }, { value: 'claimed', label: t.claimed }, { value: 'unclaimed', label: t.unclaimed }]} />}</Field>
        </PopoverContent>
      </Popover>
      {summary && <span aria-label={t.applied} title={summary} className="text-muted-foreground min-w-0 max-w-52 truncate text-xs">{summary}</span>}
      {(count > 0 || filters.search) && <Button variant="ghost" size="sm" className="h-8 px-2 text-xs" onClick={clear}>{t.clear}</Button>}
      <Picker aria-label={t.sort} value={sort} onValueChange={(value) => setSort(value as EpicsSort)}
        className="ml-auto h-8 w-auto max-w-full text-xs"
        options={[{ value: 'attention', label: t.attentionFirst }, { value: 'activity', label: t.recentActivity },
          { value: 'title', label: t.titleOrder }, { value: 'progress', label: t.progressOrder }]} />
    </div>
    {visible.length === 0 ? <div className="text-muted-foreground px-3 py-12 text-center text-sm">
      <p>{epics.length === 0 ? t.empty : t.noMatch}</p>
      {epics.length === 0 ? <p className="mt-1 text-xs">{t.emptyHint}</p>
        : <Button variant="ghost" size="sm" className="mt-2" onClick={() => setFilters({ ...DEFAULT_FILTERS, scope: 'all' })}>{t.reset}</Button>}
    </div> : <div className="border-border-soft bg-card min-h-0 min-w-0 overflow-y-auto rounded-lg border">
      <div aria-hidden className={cn('text-muted-foreground border-border-soft bg-card sticky top-0 z-10 hidden gap-5 border-b px-4 py-2 text-[11px] lg:grid', columns)}>
        <span>{t.epic}</span><span>{t.state}</span><span>{t.progress}</span><span>{t.working}</span>
      </div>
      <ul className="divide-border-soft divide-y">
        {visible.map((epic) => {
          const activity = activityAt(epic)
          const blocked = epic.by_category?.blocked ?? 0
          const warnings = epicWarnings(epic)
          const initiatives = (epic.initiatives ?? []).map((id) => laneTitles[id] ?? id).join(' · ')
          return <li key={epic.id}>
            <button type="button" onClick={() => onOpen(epic.id)}
              className={cn('hover:bg-muted focus-visible:ring-ring grid w-full min-w-0 cursor-pointer grid-cols-[minmax(0,1fr)_minmax(0,1fr)] items-center gap-x-5 gap-y-2 px-4 py-3 text-left focus-visible:ring-2 focus-visible:ring-inset focus-visible:outline-none', columns)}>
              <span className="col-span-2 min-w-0 lg:col-span-1">
                <span className="text-foreground block truncate text-[13.5px] font-semibold" title={epic.title}>{epic.title}</span>
                <span className="text-muted-foreground mt-1 flex min-w-0 gap-2 text-[11px]">
                  <span className="min-w-0 truncate" title={initiatives || t.noInitiative}>{initiatives || t.noInitiative}</span>
                  <span className="min-w-0 max-w-28 truncate font-mono opacity-75" title={epic.id}>{epic.id}</span>
                </span>
                {((epic.own_open_questions ?? 0) > 0 || epic.awaiting_answer > 0 || blocked > 0 || isStalled(epic, stalledAfter) || warnings.length > 0) &&
                  <span className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-[color:var(--warn)]">
                    {(epic.own_open_questions ?? 0) > 0 && <span className="inline-flex items-center gap-1"><MessageCircle size={12} aria-hidden />{epic.own_open_questions === 1 ? t.ownQuestion : fill(t.ownQuestions, { n: epic.own_open_questions! })}</span>}
                    {epic.awaiting_answer > 0 && <span className="inline-flex items-center gap-1"><MessageCircle size={12} aria-hidden />{epic.awaiting_answer === 1 ? t.awaitingOne : fill(t.awaiting, { n: epic.awaiting_answer })}</span>}
                    {blocked > 0 && <span className="inline-flex items-center gap-1"><Ban size={12} aria-hidden />{blocked === 1 ? t.blockedOne : fill(t.blocked, { n: blocked })}</span>}
                    {isStalled(epic, stalledAfter) && <span className="inline-flex items-center gap-1"><Clock3 size={12} aria-hidden />{fill(t.stalled, { age: fmtDuration(epic.claim?.idle_seconds ?? null) })}</span>}
                    {warnings.map((flag) => <span key={flag} className="inline-flex items-center gap-1"><AlertTriangle size={12} className="shrink-0" aria-hidden />
                      {flag === 'done_with_open_children' ? t.doneWithOpen : flag === 'open_with_all_children_done' ? t.openWithDone : sentence(flag)}
                    </span>)}
                  </span>}
              </span>
              <State epic={epic} />
              <span className="min-w-0 text-[11px] tabular-nums">
                {epic.total === 0 ? <span className="text-muted-foreground">{t.noTasks}</span> : <>
                  <span aria-hidden className="bg-secondary mb-1.5 block h-1 max-w-28 overflow-hidden rounded-full">
                    <span className="bg-primary block h-full rounded-full" style={{ width: `${Math.max(0, Math.min(100, epic.percent))}%` }} />
                  </span>
                  <span className="text-muted-foreground">{fill(t.done, { done: epic.done, total: epic.total })}</span>
                </>}
              </span>
              <span className="col-span-2 flex min-w-0 items-baseline gap-2 text-xs lg:col-span-1 lg:flex-col lg:gap-1">
                <span className="text-muted-foreground truncate" title={epic.claim?.holder}>{epic.claim?.holder ?? t.unclaimed}</span>
                <span className="text-muted-foreground truncate text-[11px] tabular-nums" title={activity ?? undefined}>
                  {activity && Number.isFinite(Date.parse(activity)) ? fill(t.updated, { age: fmtAge(activity) }) : t.unknown}
                </span>
              </span>
            </button>
          </li>
        })}
      </ul>
    </div>}
  </section>
}
