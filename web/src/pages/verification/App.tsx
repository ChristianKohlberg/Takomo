// The Tests view of the specification workspace: behaviors, the tests linked to
// them, and what the latest reported runs say. Rendered full-width as the Tests
// view and `compact` in the section side panel of Document and Map.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { affectsProjectTopic, useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { Button } from '@/components/ui/button'
import type { ApiErrorShape } from '@/lib/api'
import {
  BEHAVIOR_STATUSES,
  createBehavior,
  listBehaviors,
  patchBehavior,
  resultStamp,
  type Behavior,
  type BehaviorStatus,
  type StatusCounts,
} from '@/lib/behaviors'
import { pick } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import { useSpecification } from '../specification/context'
import { BehaviorDetail } from './BehaviorDetail'
import { BehaviorDialog } from './BehaviorDialog'
import { SectionSelect } from './SectionSelect'
import { StatusBadge, statusLabel } from './status'
import { STR } from './strings'

const EMPTY_COUNTS: StatusCounts = { total: 0, verified: 0, failing: 0, stale: 0, untested: 0 }

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
  const [version, setVersion] = useState(0)
  const epoch = useRef(0)
  const loadedScope = useRef({ token, project })

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
  useProjectUpdates(token, project, async (event) => {
    if (!affectsProjectTopic(event, 'behaviors', 'projects')) return
    setVersion((v) => v + 1)
    await refresh()
  })

  // A local change: the list, the workspace's section counts, and the open detail.
  const changed = useCallback(async () => {
    await Promise.all([refresh(), refreshVerification().catch(onError)])
  }, [refresh, refreshVerification, onError])

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
    ? (verification?.sections[section] ?? EMPTY_COUNTS)
    : (verification?.summary ?? EMPTY_COUNTS)
  const sectionTitle = (id: string | null) => (id ? (nodes.find((n) => n.id === id)?.title ?? id) : null)

  const detail = selected ? (
    <BehaviorDetail
      key={selected}
      token={token}
      id={selected}
      nodes={nodes}
      canWrite={canWrite}
      t={t}
      version={version}
      onBack={() => openBehavior(null)}
      onChanged={() => void changed()}
      onDeleted={() => {
        openBehavior(null)
        void changed()
      }}
      onError={onError}
    />
  ) : null

  return (
    <>
      <div className="flex flex-none flex-wrap items-center gap-2 border-b px-4 py-2">
        <span className="grow" />
        {canWrite && <Button onClick={() => setCreating(true)}>+ {t.newBehavior}</Button>}
        <Button variant="outline" onClick={() => void changed()}>
          {t.refresh}
        </Button>
      </div>
      <main className="min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-5">
        <div
          className={cn(
            'mx-auto grid w-full gap-4 pb-12',
            selected && !compact ? 'max-w-360 md:grid-cols-2 md:items-start' : 'max-w-240',
          )}
        >
          <div className={cn('grid min-w-0 gap-4', selected && (compact ? 'hidden' : 'hidden md:grid'))}>
            {!compact && (
              <div>
                <h1 className="text-lg font-semibold">{t.heading}</h1>
                <p className="text-muted-foreground text-sm">{t.intro}</p>
              </div>
            )}
            {section && !compact && (
              <div className="bg-muted flex flex-wrap items-center gap-2 rounded-md px-3 py-2 text-sm">
                <span className="min-w-0 break-words">{sectionTitle(section)}</span>
                <Button variant="ghost" size="sm" onClick={() => setSection(null)}>
                  {t.allSections}
                </Button>
              </div>
            )}
            {loading && <p role="status">{t.loading}</p>}
            {loadError && (
              <div role="alert" className="rounded-xl border p-4">
                <h2 className="font-semibold">
                  {loadError.permission ? t.noAccess.replace('{project}', project) : t.loadFailed}
                </h2>
                <p className="mt-2 text-sm break-words">{loadError.message}</p>
                <p className="text-muted-foreground mt-2 text-sm">
                  {loadError.permission ? t.noAccessHint : t.retryHint}
                </p>
              </div>
            )}
            {!loading && !loadError && (
              <>
                <div className="grid gap-1.5">
                  <div className="flex flex-wrap gap-2" role="group" aria-label={t.overview}>
                    {BEHAVIOR_STATUSES.map((value) => (
                      <Button
                        key={value}
                        variant={status === value ? 'secondary' : 'outline'}
                        size="sm"
                        aria-pressed={status === value}
                        onClick={() => setStatus((current) => (current === value ? '' : value))}
                      >
                        {statusLabel(value, t)} · {counts[value]}
                      </Button>
                    ))}
                  </div>
                  {verification && (
                    <p className="text-muted-foreground m-0 text-xs">
                      {t.freshness.replace('{days}', String(verification.fresh_days))}
                    </p>
                  )}
                </div>
                <div className="flex min-w-0 flex-col gap-2 md:flex-row">
                  <input
                    type="search"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    aria-label={t.search}
                    placeholder={t.search}
                    className="bg-card min-w-0 flex-1 rounded-md border px-3 py-2 text-sm"
                  />
                  {!compact && (
                    <SectionSelect
                      label={t.filterSection}
                      nodes={nodes}
                      value={section}
                      noneLabel={t.allSections}
                      onChange={setSection}
                    />
                  )}
                  {(search || status) && (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setSearch('')
                        setStatus('')
                      }}
                    >
                      {t.clearFilters}
                    </Button>
                  )}
                </div>
                {total > items.length && (
                  <p className="text-muted-foreground m-0 text-xs">
                    {t.truncated.replace('{shown}', String(items.length)).replace('{total}', String(total))}
                  </p>
                )}
                {!filtered.length && (
                  <div className="text-muted-foreground rounded-xl border border-dashed px-5 py-8 text-center text-sm">
                    {search || status ? t.emptyFiltered : section ? t.emptySection : t.empty}
                  </div>
                )}
                <ul className="m-0 grid list-none gap-2 p-0">
                  {filtered.map((b) => (
                    <li key={b.id}>
                      <button
                        type="button"
                        onClick={() => openBehavior(b.id)}
                        aria-current={b.id === selected ? 'true' : undefined}
                        className={cn(
                          'bg-card hover:bg-muted w-full min-w-0 cursor-pointer rounded-xl border p-3 text-left',
                          b.id === selected && 'border-primary',
                        )}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <h2 className="min-w-0 flex-1 font-semibold break-words">{b.title}</h2>
                          <StatusBadge status={b.status} t={t} />
                        </div>
                        <p className="text-muted-foreground m-0 mt-1 text-xs break-words">
                          {b.tests.length
                            ? `${b.tests.length} ${b.tests.length === 1 ? t.test : t.tests}`
                            : t.noTests}
                          {b.last_result && ` · ${resultStamp(b.last_result)}`}
                          {!section && b.section && ` · ${sectionTitle(b.section)}`}
                        </p>
                      </button>
                    </li>
                  ))}
                </ul>
                {!compact && !section && verification && verification.unlinked_tests.total > 0 && (
                  <UnlinkedTests
                    t={t}
                    canWrite={canWrite}
                    behaviors={items}
                    tests={verification.unlinked_tests.items}
                    total={verification.unlinked_tests.total}
                    onLink={async (behavior, test) => {
                      try {
                        await patchBehavior(token, behavior.id, { tests: [...behavior.tests, test] })
                        await changed()
                      } catch (error) {
                        onError(error)
                      }
                    }}
                  />
                )}
              </>
            )}
          </div>
          {detail && <div className="min-w-0">{detail}</div>}
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

function UnlinkedTests({
  t,
  canWrite,
  behaviors,
  tests,
  total,
  onLink,
}: {
  t: (typeof STR)['en']
  canWrite: boolean
  behaviors: Behavior[]
  tests: { test: string; outcome: 'pass' | 'fail'; at: string; commit: string | null }[]
  total: number
  onLink: (behavior: Behavior, test: string) => Promise<void>
}) {
  return (
    <section className="grid gap-2 border-t pt-4" aria-label={t.unlinked}>
      <h2 className="text-sm font-semibold">
        {t.unlinked} · {total}
      </h2>
      <p className="text-muted-foreground m-0 text-xs">{t.unlinkedHint}</p>
      <ul className="m-0 grid list-none gap-2 p-0">
        {tests.map((item) => (
          <li
            key={item.test}
            className="flex min-w-0 flex-col gap-2 rounded-md border px-3 py-2 md:flex-row md:items-center"
          >
            <div className="min-w-0 flex-1">
              <code className="text-xs break-all">{item.test}</code>
              <p className="text-muted-foreground m-0 text-xs">
                <span className={item.outcome === 'pass' ? 'text-ok' : 'text-nf'}>
                  {item.outcome === 'pass' ? t.pass : t.fail}
                </span>
                {' · '}
                {resultStamp(item)}
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
                className="bg-card min-w-0 max-w-full rounded-md border px-2 py-1 text-xs md:max-w-60"
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
    </section>
  )
}
