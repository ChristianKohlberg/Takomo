// The Promises view of the specification workspace: what the software has
// promised, whether each promise holds, and what changed lately. Rendered
// full-width as its own view and `compact` in the section side panel of
// Document and Map.
//
// Read top to bottom, it narrows: a short report of what moved in the period,
// then the promises per section in plain words. The evidence (tests), the
// track record and the edit forms only appear once a promise is opened, and
// the raw test keys only on request.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { AlertTriangle, ChevronDown, ChevronRight, Flag, Plus, Search } from 'lucide-react'
import { affectsProjectTopic, useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { Button } from '@/components/ui/button'
import type { ApiErrorShape } from '@/lib/api'
import {
  BEHAVIOR_STATUSES,
  createBehavior,
  describeTest,
  fetchReport,
  gistOf,
  listBehaviors,
  patchBehavior,
  resultStamp,
  type Behavior,
  type BehaviorStatus,
  type ReportItem,
  type ReportList,
  type Run,
  type StatusCounts,
  type VerificationReport,
} from '@/lib/behaviors'
import { fmtAge } from '@/lib/format'
import { pick } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import { useSpecification } from '../specification/context'
import { BehaviorDetail, kindLabel } from './BehaviorDetail'
import { BehaviorDialog } from './BehaviorDialog'
import { SectionSelect } from './SectionSelect'
import { STATUS_ORDER, StatusBar, StatusIcon, statusLabel, statusText } from './status'
import { STR } from './strings'

type Labels = (typeof STR)['en']
type Period = 7 | 30

const EMPTY_COUNTS: StatusCounts = { total: 0, verified: 0, failing: 0, stale: 0, untested: 0 }
/** Titles listed per report box before "and n more". */
const REPORT_SHOWN = 5

function countOf(behaviors: Behavior[]): StatusCounts {
  const counts = { ...EMPTY_COUNTS }
  for (const b of behaviors) {
    counts.total++
    counts[b.status]++
  }
  return counts
}

const fill = (text: string, values: Record<string, string | number>) =>
  text.replace(/\{(\w+)\}/g, (whole, key: string) => (key in values ? String(values[key]) : whole))

/** A section's one line: how many work, then what needs doing. */
function sectionSays(c: StatusCounts, t: Labels): string {
  if (c.total > 0 && c.verified === c.total) return fill(t.sectionAllWork, { n: c.total })
  const parts = [fill(t.sectionSays, { n: c.verified, total: c.total })]
  if (c.failing) parts.push(fill(t.sectionFailing, { n: c.failing }))
  if (c.stale) parts.push(fill(t.sectionStale, { n: c.stale }))
  if (c.untested) parts.push(fill(t.sectionUntested, { n: c.untested }))
  return parts.join(' · ')
}

/** What the report says about each promise, for the row's right-hand side. */
interface Moves {
  moved: Map<string, 'new' | 'repaired'>
  failingSince: Map<string, string>
}

function movesOf(report: VerificationReport | null): Moves {
  const moved = new Map<string, 'new' | 'repaired'>()
  const failingSince = new Map<string, string>()
  for (const i of report?.now_working.items ?? []) moved.set(i.id, 'new')
  for (const i of report?.repaired.items ?? []) moved.set(i.id, 'repaired')
  for (const i of [...(report?.broke.items ?? []), ...(report?.still_failing.items ?? [])])
    if (i.at) failingSince.set(i.id, i.at)
  return { moved, failingSince }
}

/** "since yesterday", "checked 3h ago", "new this week" — when, in the promise's terms. */
function sideNote(b: Behavior, moves: Moves, period: Period, t: Labels, now: number): string {
  const age = (at: string | undefined) => (at ? fmtAge(at, now) : '')
  switch (b.status) {
    case 'failing':
      return fill(t.sideSince, { age: age(moves.failingSince.get(b.id) ?? b.last_result?.at) })
    case 'verified': {
      const moved = moves.moved.get(b.id)
      if (moved === 'new') return period === 7 ? t.sideNew : t.sideNewMonth
      if (moved === 'repaired') return t.sideRepaired
      return fill(t.sideChecked, { age: age(b.last_result?.at) })
    }
    case 'stale':
      return fill(t.sideLast, { age: age(b.last_result?.at) })
    case 'untested':
      return b.tests.length ? fill(t.sideEvidence, { n: b.tests.length }) : t.sideNoEvidence
  }
}

/** For a promise that does not work: what the failing evidence said, in a sentence. */
function whatHappened(b: Behavior, t: Labels): string | null {
  const last = b.last_result
  if (b.status !== 'failing' || !last) return null
  if (last.detail) return last.detail
  const test = describeTest(last.test)
  return fill(t.failedEvidence, { test: test.name || kindLabel(test.kind, t) })
}

export function TestsView({ compact = false }: { compact?: boolean }) {
  const { token, lang, project, projects, scopes, nodes, verification, refreshVerification, openBehavior, onError } =
    useSpecification()
  const t = pick(STR, lang)
  // An archived project refuses behavior writes, so offer none.
  const canWrite = scopes.includes('write') && projects.find((p) => p.id === project)?.archived !== true
  const [params] = useSearchParams()
  const selected = params.get('behavior')
  const [section, setSection] = useWorkspaceSection()
  const [items, setItems] = useState<Behavior[]>([])
  const [total, setTotal] = useState(0)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<{ permission: boolean; message: string } | null>(null)
  const [search, setSearch] = useState('')
  const [status, setStatus] = useState<BehaviorStatus | ''>('')
  const [creating, setCreating] = useState(false)
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({})
  const [showUnlinked, setShowUnlinked] = useState(false)
  const [version, setVersion] = useState(0)
  const [period, setPeriod] = useState<Period>(7)
  const [report, setReport] = useState<VerificationReport | null>(null)
  const epoch = useRef(0)
  const reportEpoch = useRef(0)
  const loadedScope = useRef({ token, project })
  const selectedRow = useRef<HTMLLIElement | null>(null)

  const refresh = useCallback(async () => {
    const attempt = ++epoch.current
    const previous = loadedScope.current
    if (previous.token !== token || previous.project !== project) setLoading(true)
    loadedScope.current = { token, project }
    try {
      const page = await listBehaviors(token, project)
      if (attempt !== epoch.current) return
      setItems(page.items)
      setTotal(page.total)
      setLoadError(null)
    } catch (error) {
      if (attempt !== epoch.current) return
      const failure = error as ApiErrorShape
      if (failure.auth) onError(error)
      setLoadError({ permission: failure.status === 403, message: failure.message || '' })
    } finally {
      if (attempt === epoch.current) setLoading(false)
    }
  }, [token, project, onError])

  // The report is extra: without it the list still reads, so a failure only hides it.
  const refreshReport = useCallback(async () => {
    if (compact) return
    const attempt = ++reportEpoch.current
    try {
      const next = await fetchReport(token, project, period)
      if (attempt === reportEpoch.current) setReport(next)
    } catch (error) {
      if ((error as ApiErrorShape).auth) onError(error)
      if (attempt === reportEpoch.current) setReport(null)
    }
  }, [compact, token, project, period, onError])

  useEffect(() => {
    const counter = epoch
    void refresh()
    return () => {
      counter.current++
    }
  }, [refresh])
  useEffect(() => {
    const counter = reportEpoch
    void refreshReport()
    return () => {
      counter.current++
    }
  }, [refreshReport])
  // Live updates keep the view current; there is nothing to refresh by hand.
  useProjectUpdates(token, project, async (event) => {
    if (!affectsProjectTopic(event, 'behaviors', 'projects')) return
    setVersion((v) => v + 1)
    await Promise.all([refresh(), refreshReport()])
  })

  // A deep link (`?behavior=`) opens a row that may be far down the list.
  useEffect(() => {
    if (!loading && selected) selectedRow.current?.scrollIntoView?.({ block: 'nearest' })
  }, [loading, selected])

  // A local change: the list, the report, the workspace's section counts, and the open detail.
  const changed = useCallback(async () => {
    await Promise.all([refresh(), refreshReport(), refreshVerification().catch(onError)])
  }, [refresh, refreshReport, refreshVerification, onError])

  const now = Date.now()
  const moves = movesOf(report)
  const scoped = items.filter((b) => !section || b.section === section)
  const needle = search.trim().toLocaleLowerCase()
  const filtered = scoped.filter(
    (b) =>
      (!status || b.status === status) &&
      (!needle ||
        [b.title, b.statement, ...b.tests, nodes.find((n) => n.id === b.section)?.title ?? '']
          .join(' ')
          .toLocaleLowerCase()
          .includes(needle)),
  )
  const counts: StatusCounts = section
    ? (verification?.sections[section] ?? countOf(scoped))
    : (verification?.summary ?? countOf(items))
  const freshDays = verification?.fresh_days ?? 14
  const sectionTitle = (id: string | null) => (id ? (nodes.find((n) => n.id === id)?.title ?? id) : t.noSectionGroup)
  const filtering = Boolean(needle || status)

  // Groups in plan order, the unsectioned last; a group with a failure first.
  const planIndex = new Map(nodes.map((n, i) => [n.id, i]))
  const groupKeys = [...new Set(filtered.map((b) => b.section ?? ''))].sort((a, b) => {
    const urgent = (key: string) => (filtered.some((x) => (x.section ?? '') === key && x.status === 'failing') ? 0 : 1)
    const place = (key: string) => (key ? (planIndex.get(key) ?? nodes.length) : nodes.length + 1)
    return urgent(a) - urgent(b) || place(a) - place(b)
  })
  const groups = groupKeys.map((key) => {
    const all = items.filter((b) => (b.section ?? '') === key)
    const rows = filtered
      .filter((b) => (b.section ?? '') === key)
      .sort((a, b) => STATUS_ORDER[a.status] - STATUS_ORDER[b.status] || a.title.localeCompare(b.title))
    return { key, title: sectionTitle(key || null), counts: countOf(all), rows }
  })
  // A section with nothing left to do starts folded, unless a filter is looking into it.
  const isOpen = (key: string, groupCounts: StatusCounts, rows: Behavior[]) =>
    rows.some((b) => b.id === selected) ||
    (collapsed[key] ?? (!filtering && groupCounts.total > 0 && groupCounts.verified === groupCounts.total)) === false

  const chips: { value: BehaviorStatus | ''; label: string; count: number }[] = [
    { value: '', label: t.all, count: counts.total },
    ...BEHAVIOR_STATUSES.map((value) => ({ value, label: statusLabel(value, t), count: counts[value] })),
  ]

  return (
    <>
      <div className="bg-background sticky top-0 z-10 flex flex-none flex-wrap items-center gap-2 border-b px-4 py-2">
        <div
          className="-mx-1 flex min-w-0 flex-[1_1_100%] gap-1.5 overflow-x-auto px-1 md:flex-[0_1_auto]"
          role="group"
          aria-label={t.overview}
        >
          {chips.map((chip) => {
            const active = status === chip.value
            return (
              <button
                key={chip.value || 'all'}
                type="button"
                aria-pressed={active}
                onClick={() => setStatus(active && chip.value ? '' : chip.value)}
                className={cn(
                  'inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs whitespace-nowrap',
                  active ? 'border-primary bg-primary/10 text-foreground' : 'bg-card text-muted-foreground hover:text-foreground',
                )}
              >
                {chip.value && <StatusIcon status={chip.value} className="size-3.5" />}
                {chip.label}
                <span className="text-foreground font-semibold tabular-nums">{chip.count}</span>
              </button>
            )
          })}
        </div>
        <span className="hidden grow md:block" />
        {!compact && (
          <SectionSelect
            label={t.filterSection}
            nodes={nodes}
            value={section}
            noneLabel={t.allSections}
            onChange={setSection}
            className="h-8 min-w-0 flex-1 text-xs md:w-44 md:flex-none"
          />
        )}
        <label className="bg-card relative flex h-8 min-w-0 flex-1 items-center rounded-md border md:w-52 md:flex-none">
          <Search className="text-muted-foreground absolute left-2 size-3.5" aria-hidden="true" />
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            aria-label={t.search}
            placeholder={t.search}
            className="h-full w-full min-w-0 rounded-md bg-transparent pr-2 pl-7 text-xs outline-none"
          />
        </label>
        {canWrite && (
          <Button size="sm" onClick={() => setCreating(true)} aria-label={t.newBehavior}>
            <Plus className="size-4" aria-hidden="true" />
            <span className="hidden sm:inline">{t.newBehavior}</span>
          </Button>
        )}
      </div>
      <main className="min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-5">
        <div className="mx-auto grid w-full max-w-240 gap-6 pb-12">
          {loading && <p role="status">{t.loading}</p>}
          {loadError && (
            <div role="alert" className="rounded-xl border p-4">
              <h2 className="font-semibold">
                {loadError.permission ? t.noAccess.replace('{project}', project) : t.loadFailed}
              </h2>
              <p className="mt-2 text-sm break-words">{loadError.message}</p>
              <p className="text-muted-foreground mt-2 text-sm">{loadError.permission ? t.noAccessHint : t.retryHint}</p>
            </div>
          )}
          {!loading && !loadError && (
            <>
              {!compact && items.length > 0 && (
                <Report
                  t={t}
                  counts={counts}
                  report={report}
                  period={period}
                  onPeriod={setPeriod}
                  run={verification?.latest_run ?? null}
                  freshDays={freshDays}
                  now={now}
                  section={section}
                  sectionTitle={sectionTitle}
                  onClearSection={() => setSection(null)}
                  onOpen={(id) => openBehavior(id)}
                />
              )}
              {total > items.length && (
                <p className="text-muted-foreground m-0 text-xs">
                  {t.truncated.replace('{shown}', String(items.length)).replace('{total}', String(total))}
                </p>
              )}
              {!filtered.length && (
                <div className="text-muted-foreground rounded-xl border border-dashed px-5 py-8 text-center text-sm">
                  {items.length === 0 && !compact && <p className="text-foreground m-0 mb-1 font-semibold">{t.heading}</p>}
                  {filtering ? t.emptyFiltered : section ? t.emptySection : t.empty}
                  {filtering && (
                    <div className="mt-2">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSearch('')
                          setStatus('')
                        }}
                      >
                        {t.clearFilters}
                      </Button>
                    </div>
                  )}
                </div>
              )}
              <div className="grid gap-5">
                {groups.map((group) => {
                  // A lone filtered section has no header to unfold it with, so it is always open.
                  const headed = !(section && groups.length === 1)
                  const open = !headed || isOpen(group.key, group.counts, group.rows)
                  const done = group.counts.total > 0 && group.counts.verified === group.counts.total
                  return (
                    <section key={group.key || 'none'} aria-label={group.title} className="grid gap-1">
                      {headed && (
                        <button
                          type="button"
                          aria-expanded={open}
                          aria-label={t.toggleSection.replace('{section}', group.title)}
                          onClick={() => setCollapsed((c) => ({ ...c, [group.key]: open }))}
                          className="flex min-w-0 cursor-pointer items-start gap-2 pb-1 text-left"
                        >
                          {open ? (
                            <ChevronDown className="text-muted-foreground mt-0.5 size-4 shrink-0" aria-hidden="true" />
                          ) : (
                            <ChevronRight className="text-muted-foreground mt-0.5 size-4 shrink-0" aria-hidden="true" />
                          )}
                          <span className="min-w-0 flex-1">
                            <h2 className="m-0 text-base font-semibold break-words">{group.title}</h2>
                            <span className={cn('block text-xs', done ? 'text-ok' : 'text-muted-foreground')}>
                              {sectionSays(group.counts, t)}
                            </span>
                          </span>
                          <StatusBar counts={group.counts} label={group.title} className="mt-1.5 w-16 shrink-0 md:w-24" />
                        </button>
                      )}
                      {open && (
                        <ul className="m-0 grid list-none gap-0 border-t p-0">
                          {group.rows.map((b) => {
                            const expanded = b.id === selected
                            const alarm = whatHappened(b, t)
                            const gist = gistOf(b.statement)
                            return (
                              <li
                                key={b.id}
                                ref={expanded ? selectedRow : undefined}
                                className={cn('min-w-0 border-b', expanded && 'bg-card')}
                              >
                                <button
                                  type="button"
                                  aria-expanded={expanded}
                                  onClick={() => openBehavior(expanded ? null : b.id)}
                                  className="hover:bg-muted/60 flex w-full min-w-0 cursor-pointer items-start gap-3 px-2 py-3 text-left"
                                >
                                  <StatusIcon status={b.status} label={statusLabel(b.status, t)} className="mt-0.5 size-[18px]" />
                                  <span className="flex min-w-0 flex-1 flex-col gap-0.5 sm:flex-row sm:gap-4">
                                    <span className="min-w-0 flex-1">
                                      <span className="block text-sm font-semibold break-words">{b.title}</span>
                                      {gist && gist !== b.title && !expanded && (
                                        <span className="text-muted-foreground line-clamp-2 block text-[13px] break-words">
                                          {gist}
                                        </span>
                                      )}
                                    </span>
                                    <span className="shrink-0 text-xs sm:text-right">
                                      <span className={cn('font-semibold', statusText[b.status])}>
                                        {statusLabel(b.status, t)}
                                      </span>
                                      <span className="text-muted-foreground sm:block">
                                        <span className="sm:hidden"> · </span>
                                        {sideNote(b, moves, period, t, now)}
                                      </span>
                                    </span>
                                  </span>
                                </button>
                                {alarm && !expanded && (
                                  <p className="bg-nfbg m-0 mx-2 mb-3 ml-9 rounded-md px-2.5 py-1.5 text-xs break-words">
                                    <span className="text-nf font-semibold">{t.whatHappened}</span> {alarm}
                                  </p>
                                )}
                                {expanded && (
                                  <div className="px-2 pt-1 pb-5 md:pl-9">
                                    <BehaviorDetail
                                      key={b.id}
                                      token={token}
                                      project={project}
                                      id={b.id}
                                      nodes={nodes}
                                      canWrite={canWrite}
                                      t={t}
                                      freshDays={freshDays}
                                      version={version}
                                      onChanged={() => void changed()}
                                      onDeleted={() => {
                                        openBehavior(null)
                                        void changed()
                                      }}
                                      onError={onError}
                                    />
                                  </div>
                                )}
                              </li>
                            )
                          })}
                        </ul>
                      )}
                    </section>
                  )
                })}
              </div>
              {!compact && !section && verification && verification.unlinked_tests.total > 0 && (
                <section className="grid gap-2 border-t pt-4" aria-label={t.unlinked.replace('{n}', String(verification.unlinked_tests.total))}>
                  <button
                    type="button"
                    aria-expanded={showUnlinked}
                    onClick={() => setShowUnlinked((v) => !v)}
                    className="text-warn inline-flex cursor-pointer items-center gap-1.5 justify-self-start text-sm"
                  >
                    <AlertTriangle className="size-4" aria-hidden="true" />
                    {t.unlinked.replace('{n}', String(verification.unlinked_tests.total))}
                    <ChevronRight className={cn('size-4 transition-transform', showUnlinked && 'rotate-90')} aria-hidden="true" />
                  </button>
                  {showUnlinked && (
                    <UnlinkedTests
                      t={t}
                      canWrite={canWrite}
                      behaviors={items}
                      tests={verification.unlinked_tests.items}
                      onLink={async (behavior, test) => {
                        try {
                          await patchBehavior(token, behavior.id, { add_tests: [test] })
                          await changed()
                        } catch (error) {
                          onError(error)
                        }
                      }}
                    />
                  )}
                </section>
              )}
            </>
          )}
        </div>
      </main>
      <BehaviorDialog
        open={creating}
        onOpenChange={setCreating}
        nodes={nodes}
        defaultSection={section}
        labels={t}
        onSubmit={async (fields) => {
          const created = await createBehavior(token, project, fields)
          await changed()
          openBehavior(created.id)
        }}
      />
    </>
  )
}

/** Verified promises per period as a small line, the newest point marked. */
function Trend({ points, label }: { points: { verified: number; total: number }[]; label: string }) {
  const width = 120
  const height = 32
  const top = Math.max(1, ...points.map((p) => p.total))
  const x = (i: number) => 3 + (i * (width - 6)) / Math.max(1, points.length - 1)
  const y = (v: number) => height - 3 - (v / top) * (height - 6)
  const line = points.map((p, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)} ${y(p.verified).toFixed(1)}`).join(' ')
  const last = points.length - 1
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} className="shrink-0">
      <line x1="3" x2={width - 3} y1={height - 3} y2={height - 3} className="stroke-border" />
      <path
        d={`${line} L${x(last).toFixed(1)} ${height - 3} L${x(0).toFixed(1)} ${height - 3} Z`}
        className="fill-okbg"
      />
      <path d={line} fill="none" className="stroke-ok" strokeWidth="2" strokeLinejoin="round" />
      <circle cx={x(last)} cy={y(points[last]?.verified ?? 0)} r="3" className="fill-ok" />
    </svg>
  )
}

/**
 * The first thing to read: how many promises work, what moved in the period
 * — newly working, repaired, broken, by name — and sections that became
 * complete. Narrowed to one section, the lists narrow with it.
 */
function Report({
  t,
  counts,
  report,
  period,
  onPeriod,
  run,
  freshDays,
  now,
  section,
  sectionTitle,
  onClearSection,
  onOpen,
}: {
  t: Labels
  counts: StatusCounts
  report: VerificationReport | null
  period: Period
  onPeriod: (period: Period) => void
  run: Run | null
  freshDays: number
  now: number
  section: string | null
  sectionTitle: (id: string | null) => string
  onClearSection: () => void
  onOpen: (id: string) => void
}) {
  const inScope = (list: ReportList | undefined) =>
    (list?.items ?? []).filter((i) => !section || i.section === section)
  // Narrowed to a section the list is what came back; the whole project uses the total.
  const sizeOf = (list: ReportList | undefined) => (section ? inScope(list).length : (list?.total ?? 0))
  const working = inScope(report?.now_working)
  const repaired = inScope(report?.repaired)
  const broke = inScope(report?.broke)
  const stillFailing = sizeOf(report?.still_failing)
  const wentStale = sizeOf(report?.went_stale)
  const completed = (report?.sections_completed ?? []).filter((c) => !section || c.section === section)
  const moved = working.length + repaired.length + broke.length > 0
  const ago = period === 7 ? t.periodAgoWeek : t.periodAgoMonth
  const delta = report && !section ? report.now.verified - report.then.verified : null
  const trend = report && !section ? report.trend : null

  return (
    <section className="grid gap-4" aria-label={t.reportLabel}>
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <div className="bg-muted inline-flex rounded-md p-0.5" role="group" aria-label={t.reportLabel}>
          {([7, 30] as const).map((p) => (
            <button
              key={p}
              type="button"
              aria-pressed={period === p}
              onClick={() => onPeriod(p)}
              className={cn(
                'cursor-pointer rounded px-2 py-0.5',
                period === p ? 'bg-background text-foreground font-semibold shadow-sm' : 'text-muted-foreground',
              )}
            >
              {p === 7 ? t.periodWeek : t.periodMonth}
            </button>
          ))}
        </div>
        {section && (
          <button
            type="button"
            onClick={onClearSection}
            className="text-muted-foreground hover:text-foreground inline-flex cursor-pointer items-center gap-1"
          >
            {sectionTitle(section)} · {t.allSections} ×
          </button>
        )}
      </div>

      <div className="flex flex-wrap items-end gap-x-6 gap-y-2">
        <div className="grid min-w-0 gap-1">
          <p className="m-0 text-2xl leading-tight font-semibold tracking-tight">
            <span className="text-ok">
              {fill(section ? t.headlineSection : t.headline, { n: counts.verified, total: counts.total })}
            </span>
            {delta !== null && (
              <span className="text-foreground">
                {', '}
                {delta > 0
                  ? fill(t.deltaUp, { n: delta, period: ago })
                  : delta < 0
                    ? fill(t.deltaDown, { n: -delta, period: ago })
                    : fill(t.deltaSame, { period: ago })}
              </span>
            )}
          </p>
        </div>
        {trend && trend.length > 1 && (
          <div className="text-muted-foreground flex items-center gap-2 text-xs tabular-nums">
            <Trend
              points={trend}
              label={fill(t.trendLabel, { values: trend.map((p) => p.verified).join(' → ') })}
            />
            <span aria-hidden="true">{trend.map((p) => p.verified).join(' → ')}</span>
          </div>
        )}
      </div>
      <StatusBar counts={counts} label={t.reportLabel} className="h-2" />

      {report && (
        <>
          {moved ? (
            <div className="grid gap-2 md:grid-cols-3">
              {working.length > 0 && (
                <MoveBox tone="up" count={working.length} label={t.movedWorking} items={working} t={t} onOpen={onOpen} />
              )}
              {repaired.length > 0 && (
                <MoveBox tone="fix" count={repaired.length} label={t.movedRepaired} items={repaired} t={t} onOpen={onOpen} />
              )}
              {broke.length > 0 && (
                <MoveBox
                  tone="down"
                  count={broke.length}
                  label={t.movedBroke}
                  items={broke}
                  t={t}
                  onOpen={onOpen}
                  note={(i) => (i.at ? fill(t.since, { age: fmtAge(i.at, now) }) : '')}
                />
              )}
            </div>
          ) : (
            <p className="text-muted-foreground m-0 text-sm">{t.nothingMoved}</p>
          )}
          {(stillFailing > 0 || wentStale > 0) && (
            <p className="text-muted-foreground m-0 text-xs">
              {[
                stillFailing > 0 && fill(t.movedStillFailing, { n: stillFailing }),
                wentStale > 0 && fill(t.movedStale, { n: wentStale }),
              ]
                .filter(Boolean)
                .join(' · ')}
            </p>
          )}
          {completed.map((c) => (
            <div key={c.section} className="border-ok/60 flex items-center gap-2.5 rounded-lg border border-dashed px-3 py-2 text-sm">
              <Flag className="text-ok size-4 shrink-0" aria-hidden="true" />
              <p className="m-0 min-w-0">
                <span className="font-semibold">{fill(t.milestone, { section: sectionTitle(c.section), n: c.total })}</span>{' '}
                <span className="text-muted-foreground">{t.milestoneHint}</span>
              </p>
            </div>
          ))}
        </>
      )}

      <p className="text-muted-foreground m-0 text-xs">
        {run ? (
          <>
            {t.latestRun.replace('{age}', fmtAge(run.at, now))}
            {` · ${t.latestRunBy.replace('{actor}', run.actor)}`}
          </>
        ) : (
          t.noRun
        )}
        {' · '}
        {t.freshness.replace('{days}', String(freshDays))}
      </p>
    </section>
  )
}

const moveTone = {
  up: { box: 'bg-okbg', head: 'text-ok' },
  fix: { box: 'bg-muted', head: 'text-foreground' },
  down: { box: 'bg-nfbg', head: 'text-nf' },
}

function MoveBox({
  tone,
  count,
  label,
  items,
  t,
  onOpen,
  note,
}: {
  tone: keyof typeof moveTone
  count: number
  label: string
  items: ReportItem[]
  t: Labels
  onOpen: (id: string) => void
  note?: (item: ReportItem) => string
}) {
  return (
    <div className={cn('grid content-start gap-1.5 rounded-lg px-3 py-2.5', moveTone[tone].box)}>
      <h3 className={cn('m-0 flex items-baseline gap-1.5 text-xs font-semibold', moveTone[tone].head)}>
        <span className="text-lg leading-none tabular-nums">{count}</span>
        {label}
      </h3>
      <ul className="m-0 grid list-none gap-0.5 p-0 text-[13px]">
        {items.slice(0, REPORT_SHOWN).map((item) => (
          <li key={item.id} className="min-w-0">
            <button
              type="button"
              onClick={() => onOpen(item.id)}
              className="cursor-pointer text-left break-words hover:underline"
            >
              {item.title}
            </button>
            {note?.(item) && <span className="text-muted-foreground"> – {note(item)}</span>}
          </li>
        ))}
        {items.length > REPORT_SHOWN && (
          <li className="text-muted-foreground">{fill(t.more, { n: items.length - REPORT_SHOWN })}</li>
        )}
      </ul>
    </div>
  )
}

function UnlinkedTests({
  t,
  canWrite,
  behaviors,
  tests,
  onLink,
}: {
  t: Labels
  canWrite: boolean
  behaviors: Behavior[]
  tests: { test: string; outcome: 'pass' | 'fail'; at: string; commit: string | null }[]
  onLink: (behavior: Behavior, test: string) => Promise<void>
}) {
  return (
    <div className="grid gap-2">
      <p className="text-muted-foreground m-0 text-xs">{t.unlinkedHint}</p>
      <ul className="m-0 grid list-none gap-2 p-0">
        {tests.map((item) => (
          <li
            key={item.test}
            className="flex min-w-0 flex-col gap-2 rounded-md border px-3 py-2 md:flex-row md:items-center"
          >
            <div className="min-w-0 flex-1">
              <p className="m-0 text-sm break-words">{describeTest(item.test).name || item.test}</p>
              <p className="text-muted-foreground m-0 text-xs">
                <span className={item.outcome === 'pass' ? 'text-ok' : 'text-nf'}>
                  {item.outcome === 'pass' ? t.pass : t.fail}
                </span>
                {' · '}
                {resultStamp(item)}
                {' · '}
                <code className="break-all">{item.test}</code>
              </p>
            </div>
            {canWrite && behaviors.length > 0 && (
              <select
                aria-label={t.linkToLabel.replace('{test}', item.test)}
                value=""
                onChange={(event) => {
                  const target = behaviors.find((b) => b.id === event.target.value)
                  if (target) void onLink(target, item.test)
                }}
                className="bg-card max-w-full min-w-0 rounded-md border px-2 py-1 text-xs md:max-w-60"
              >
                <option value="">{t.linkTo}</option>
                {behaviors.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.title}
                  </option>
                ))}
              </select>
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}
