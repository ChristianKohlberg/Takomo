// The Tests view of the specification workspace: behaviors, the tests linked to
// them, and what the latest reported runs say. Rendered full-width as the Tests
// view and `compact` in the section side panel of Document and Map.
//
// Read top to bottom, it narrows: where things stand overall, then per section,
// then per behavior in plain words. Test keys, runs and edit forms only appear
// once a behavior is opened, and the raw keys only on request.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { AlertTriangle, ChevronDown, ChevronRight, Plus, Search } from 'lucide-react'
import { affectsProjectTopic, useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { Button } from '@/components/ui/button'
import type { ApiErrorShape } from '@/lib/api'
import {
  BEHAVIOR_STATUSES,
  createBehavior,
  describeTest,
  listBehaviors,
  patchBehavior,
  resultStamp,
  type Behavior,
  type BehaviorStatus,
  type Run,
  type StatusCounts,
} from '@/lib/behaviors'
import { fmtAge } from '@/lib/format'
import { pick } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import { useSpecification } from '../specification/context'
import { BehaviorDetail } from './BehaviorDetail'
import { BehaviorDialog } from './BehaviorDialog'
import { SectionSelect } from './SectionSelect'
import { STATUS_ORDER, StatusBar, StatusIcon, statusLabel, statusText } from './status'
import { STR } from './strings'

type Labels = (typeof STR)['en']

const EMPTY_COUNTS: StatusCounts = { total: 0, verified: 0, failing: 0, stale: 0, untested: 0 }

function countOf(behaviors: Behavior[]): StatusCounts {
  const counts = { ...EMPTY_COUNTS }
  for (const b of behaviors) {
    counts.total++
    counts[b.status]++
  }
  return counts
}

/** The row's one line: what a reader needs to know without opening it. */
function summaryOf(b: Behavior, t: Labels, now: number): string {
  if (!b.tests.length) return t.sumNoTests
  const age = b.last_result ? fmtAge(b.last_result.at, now) : ''
  switch (b.status) {
    case 'failing':
      return t.sumFailing.replace('{test}', b.last_result ? describeTest(b.last_result.test).name : '')
    case 'verified':
      return t.sumVerified.replace('{age}', age)
    case 'stale':
      return t.sumStale.replace('{age}', age)
    case 'untested':
      return t.sumUntested
  }
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
  const epoch = useRef(0)
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

  useEffect(() => {
    const counter = epoch
    void refresh()
    return () => {
      counter.current++
    }
  }, [refresh])
  // Live updates keep the view current; there is nothing to refresh by hand.
  useProjectUpdates(token, project, async (event) => {
    if (!affectsProjectTopic(event, 'behaviors', 'projects')) return
    setVersion((v) => v + 1)
    await refresh()
  })

  // A deep link (`?behavior=`) opens a row that may be far down the list.
  useEffect(() => {
    if (!loading && selected) selectedRow.current?.scrollIntoView?.({ block: 'nearest' })
  }, [loading, selected])

  // A local change: the list, the workspace's section counts, and the open detail.
  const changed = useCallback(async () => {
    await Promise.all([refresh(), refreshVerification().catch(onError)])
  }, [refresh, refreshVerification, onError])

  const now = Date.now()
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
        <div className="mx-auto grid w-full max-w-240 gap-5 pb-12">
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
                <Overview
                  t={t}
                  counts={counts}
                  run={verification?.latest_run ?? null}
                  freshDays={freshDays}
                  now={now}
                  section={section ? sectionTitle(section) : null}
                  onClearSection={() => setSection(null)}
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
              <div className="grid gap-4">
                {groups.map((group) => {
                  // A lone filtered section has no header to unfold it with, so it is always open.
                  const headed = !(section && groups.length === 1)
                  const open = !headed || isOpen(group.key, group.counts, group.rows)
                  return (
                    <section key={group.key || 'none'} aria-label={group.title} className="grid gap-2">
                      {headed && (
                        <button
                          type="button"
                          aria-expanded={open}
                          aria-label={t.toggleSection.replace('{section}', group.title)}
                          onClick={() => setCollapsed((c) => ({ ...c, [group.key]: open }))}
                          className="flex min-w-0 cursor-pointer items-center gap-2 text-left"
                        >
                          {open ? (
                            <ChevronDown className="text-muted-foreground size-4 shrink-0" aria-hidden="true" />
                          ) : (
                            <ChevronRight className="text-muted-foreground size-4 shrink-0" aria-hidden="true" />
                          )}
                          <h2 className="m-0 min-w-0 flex-1 truncate text-sm font-semibold">{group.title}</h2>
                          <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
                            {t.sectionProgress
                              .replace('{n}', String(group.counts.verified))
                              .replace('{total}', String(group.counts.total))}
                          </span>
                          <StatusBar counts={group.counts} label={group.title} className="w-16 shrink-0 md:w-24" />
                        </button>
                      )}
                      {open && (
                        <ul className="m-0 grid list-none gap-1.5 p-0">
                          {group.rows.map((b) => {
                            const expanded = b.id === selected
                            return (
                              <li
                                key={b.id}
                                ref={expanded ? selectedRow : undefined}
                                className={cn(
                                  'bg-card min-w-0 rounded-lg border',
                                  expanded && 'border-primary/50 shadow-sm',
                                )}
                              >
                                <button
                                  type="button"
                                  aria-expanded={expanded}
                                  onClick={() => openBehavior(expanded ? null : b.id)}
                                  className="hover:bg-muted/60 flex w-full min-w-0 cursor-pointer items-start gap-2.5 rounded-lg px-3 py-2.5 text-left"
                                >
                                  <StatusIcon status={b.status} label={statusLabel(b.status, t)} className="mt-0.5" />
                                  <span className="min-w-0 flex-1">
                                    <span className="block text-sm font-medium break-words">{b.title}</span>
                                    <span
                                      className={cn(
                                        'block text-xs break-words',
                                        b.status === 'failing' ? statusText.failing : 'text-muted-foreground',
                                      )}
                                    >
                                      {summaryOf(b, t, now)}
                                    </span>
                                  </span>
                                  <span className="text-muted-foreground mt-0.5 hidden shrink-0 text-xs sm:inline">
                                    {b.tests.length} {b.tests.length === 1 ? t.test : t.tests}
                                  </span>
                                  <ChevronDown
                                    className={cn(
                                      'text-muted-foreground mt-0.5 size-4 shrink-0 transition-transform',
                                      !expanded && '-rotate-90',
                                    )}
                                    aria-hidden="true"
                                  />
                                </button>
                                {expanded && (
                                  <div className="border-t px-3 pt-3 pb-3 md:pl-9">
                                    <BehaviorDetail
                                      key={b.id}
                                      token={token}
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

/** The first thing to read: how much is verified, and when anything last reported. */
function Overview({
  t,
  counts,
  run,
  freshDays,
  now,
  section,
  onClearSection,
}: {
  t: Labels
  counts: StatusCounts
  run: Run | null
  freshDays: number
  now: number
  section: string | null
  onClearSection: () => void
}) {
  return (
    <section className="grid gap-2" aria-label={t.progressLabel}>
      {section && (
        <button
          type="button"
          onClick={onClearSection}
          className="text-muted-foreground hover:text-foreground inline-flex cursor-pointer items-center gap-1 justify-self-start text-xs"
        >
          {section} · {t.allSections} ×
        </button>
      )}
      <p className="m-0 text-xl font-semibold tracking-tight">
        {t.progress.replace('{n}', String(counts.verified)).replace('{total}', String(counts.total))}
      </p>
      <StatusBar counts={counts} label={t.progressLabel} className="h-2.5" />
      <p className="text-muted-foreground m-0 text-xs">
        {run ? (
          <>
            {t.latestRun.replace('{age}', fmtAge(run.at, now))}
            {run.commit && ` · ${run.commit.slice(0, 7)}`}
            {` · ${t.latestRunBy.replace('{actor}', run.actor)}`}
            {' — '}
            <span className={run.failed > 0 ? 'text-nf' : undefined}>
              {t.latestRunResult.replace('{passed}', String(run.passed)).replace('{failed}', String(run.failed))}
            </span>
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
              <p className="m-0 text-sm break-words">{describeTest(item.test).name}</p>
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
