import { useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceNavigate } from '@/hooks/useWorkspace'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { useSpecification } from '../specification/context'
// /verification — whether the tests a feature was agreed on still pass.
//
// Grouped by INITIATIVE, because that is where the agreement was made and where
// the question gets asked months later. Checks nobody filed under one get their
// own group rather than being hidden: "what did we agree and never write down"
// is exactly the gap worth seeing.
//
// The gate and the worklist sit at the top as one strip. The gate answers "can
// this ship", the worklist answers "who has to do something" — and the split
// between agent and human work is the product, not a formatting choice: a
// hundred cases cost an agent minutes and cost a person most of a day.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'

import { Hint } from '@/components/Hint'
import { useToast } from '@/components/Toaster'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { CheckCard } from '@/components/verification/CheckCard'
import { CheckDialog } from '@/components/verification/CheckDialog'
import { pick } from '@/lib/i18n'
import { listInitiatives } from '@/lib/initiatives'
import { mapLink } from '@/lib/plan-url'
import {
  archiveCheck,
  createCheck,
  fetchGate,
  fetchWorklist,
  listCases,
  listChecks,
  listEnvironments,
  recordVerdict,
  type CaseRow,
  type Check,
  type CheckFields,
  type Environment,
  type Gate,
  type Worklist,
} from '@/lib/verification'
import { STR } from './strings'

/**
 * Group key for checks nobody filed under an initiative.
 *
 * Not a control character and not an empty string: an initiative id is always
 * `ini-…`, so this cannot collide with one, and it stays greppable.
 */
const UNASSIGNED = '__unassigned__'

export function TestsView({ compact = false }: { compact?: boolean }) {
  const {
    token,
    lang,
    project,
    scopes,
    checks,
    nodes: planNodes,
    map,
    setChecks,
    refreshChecks,
    editCheck,
    onError: handleErr,
  } = useSpecification()
  const navigate = useWorkspaceNavigate()
  const { toast } = useToast()
  const [initiativeTitles, setInitiativeTitles] = useState<Record<string, string>>({})
  const [worklist, setWorklist] = useState<Worklist | null>(null)
  const [gate, setGate] = useState<Gate | null>(null)
  const [environments, setEnvironments] = useState<Environment[]>([])
  const [cases, setCases] = useState<Record<string, CaseRow[]>>({})
  const [loadingCases, setLoadingCases] = useState<Record<string, boolean>>({})
  const [section, setNodeFilter] = useWorkspaceSection()
  const nodeFilter = section ?? ''
  const [creating, setCreating] = useState(false)
  const [createUnder, setCreateUnder] = useState<string | undefined>(undefined)

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = scopes.includes('write')
  const canApprove = scopes.includes('human')

  const listEpoch = useRef(0)
  useEffect(() => {
    const epoch = listEpoch
    return () => {
      epoch.current++
    }
  }, [token, project])
  const fetchAll = useCallback(async () => {
    const epoch = ++listEpoch.current
    await refreshChecks()
    if (epoch !== listEpoch.current) return
    if (!project) {
      setChecks([])
      setWorklist(null)
      setGate(null)
      return
    }
    const [w, g, inis, envs] = await Promise.all([
      // The reports are a header, not the content: a soft failure in either
      // must not take the checks down with it.
      fetchWorklist(token, project).catch(() => null),
      fetchGate(token, project).catch(() => null),
      listInitiatives(token, { project }).catch(() => ({ items: [] })),
      // Only to populate the "where must it pass" picker — a project with none
      // still shows the page, it just cannot declare environments yet.
      listEnvironments(token, project).catch(() => ({ items: [] as Environment[] })),

    ])
    if (epoch !== listEpoch.current) return
    setWorklist(w)
    setGate(g)
    const titles: Record<string, string> = {}
    for (const i of inis.items) titles[i.id] = i.title
    setInitiativeTitles(titles)
    setEnvironments(envs.items.filter((e) => !e.archived_at))
  }, [refreshChecks, project, token, setChecks])

  const openCases = useRef(cases)
  useLayoutEffect(() => { openCases.current = cases }, [cases])
  const refreshView = useCallback(async () => {
    try {
      const epoch = listEpoch.current + 1
      await fetchAll()
      if (epoch !== listEpoch.current) return
      const entries = await Promise.all(
        Object.keys(openCases.current).map(async (id) => [id, (await listCases(token, id)).items] as const),
      )
      if (epoch !== listEpoch.current) return
      setCases((current) => {
        const next = { ...current }
        for (const [id, items] of entries) if (current[id] !== undefined) next[id] = items
        return next
      })
    } catch (error) {
      handleErr(error)
    }
  }, [fetchAll, token, handleErr])
  useEffect(() => {
    if (token) void refreshView()
  }, [token, refreshView])
  useProjectUpdates(token, project, refreshView)

  const toggleCases = useCallback(
    async (check: string) => {
      if (cases[check] !== undefined) {
        setCases((c) => {
          const next = { ...c }
          delete next[check]
          return next
        })
        return
      }
      setLoadingCases((l) => ({ ...l, [check]: true }))
      try {
        const page = await listCases(token, check)
        setCases((c) => ({ ...c, [check]: page.items }))
      } catch (e) {
        handleErr(e)
      } finally {
        setLoadingCases((l) => ({ ...l, [check]: false }))
      }
    },
    [cases, token, handleErr],
  )

  const onVerdict = useCallback(
    async (
      check: string,
      caseId: string,
      verdict: 'pass' | 'fail',
      opts: { note?: string; human?: boolean; environment?: string },
    ) => {
      if (opts.human && !canApprove) {
        toast(t.needHuman, 'err')
        return
      }
      if (!canWrite) {
        toast(t.needWrite, 'err')
        return
      }
      try {
        await recordVerdict(token, caseId, verdict, opts)
        toast(t.recorded, 'success')
        // The verdict changed this check's counts AND the worklist and gate
        // above, so both are refetched rather than patched locally — a header
        // that disagrees with the row under it is worse than a slow header.
        const [page, w, g] = await Promise.all([
          listCases(token, check),
          project ? fetchWorklist(token, project).catch(() => null) : Promise.resolve(null),
          project ? fetchGate(token, project).catch(() => null) : Promise.resolve(null),
        ])
        setCases((c) => ({ ...c, [check]: page.items }))
        setWorklist(w)
        setGate(g)
        if (project) setChecks((await listChecks(token, project)).items)
      } catch (e) {
        handleErr(e)
      }
    },
    [
      canApprove,
      canWrite,
      toast,
      t.needHuman,
      t.needWrite,
      t.recorded,
      token,
      project,
      setChecks,
      handleErr,
    ],
  )

  const nodeTitles = new Map(planNodes.map((n) => [n.id, { ...n, mindmap: map?.id ?? '' }]))
  const filtered = nodeFilter ? checks.filter((c) => c.node === nodeFilter) : checks

  // Group by initiative, with the unassigned bucket last: it is a finding, but
  // it is not the thing you came to read.
  const groups = new Map<string, Check[]>()
  for (const c of filtered) {
    const key = c.initiative ?? UNASSIGNED
    const list = groups.get(key)
    if (list) list.push(c)
    else groups.set(key, [c])
  }
  const ordered = [...groups.entries()].sort(([a], [b]) => {
    if (a === UNASSIGNED) return 1
    if (b === UNASSIGNED) return -1
    return (initiativeTitles[a] ?? a).localeCompare(initiativeTitles[b] ?? b)
  })

  const cardLabels = {
    stateFailed: t.stateFailed,
    stateStale: t.stateStale,
    stateNever: t.stateNever,
    stateBlocked: t.stateBlocked,
    stateUnreachable: t.stateUnreachable,
    stateVerified: t.stateVerified,
    stateApproved: t.stateApproved,
    stateNone: t.stateNone,
    orphanGlobs: t.orphanGlobs,
    showCases: t.showCases,
    hideCases: t.hideCases,
    noCases: t.noCases,
    approve: t.approve,
    markPass: t.markPass,
    markFail: t.markFail,
    notePlaceholder: t.notePlaceholder,
    archiveCheck: t.archiveCheck,
    verifiesSection: t.verifiesSection,
    openOnMap: t.openOnMap,
  }

  return (
    <>
      <div className="flex flex-none items-center gap-2 border-b px-4 py-2">
        <Button
          onClick={() => {
            if (!canWrite) {
              toast(t.needWrite, 'err')
              return
            }
            setCreateUnder(undefined)
            setCreating(true)
          }}
        >
          + {t.newCheck}
        </Button>
        <Hint text={t.refresh}>
          <Button variant="outline" size="icon" onClick={() => fetchAll().catch(handleErr)}>
            ↻
          </Button>
        </Hint>
      </div>
      <main className="min-h-0 flex-1 overflow-y-auto px-4 pt-4.5 pb-15 md:px-5">
        <div className="mx-auto flex w-full max-w-240 flex-col gap-3.5">
          {!compact && (gate || worklist) && (
            <Card size="sm" className="gap-2 px-(--card-spacing)">
              {gate && (
                <div className="flex flex-wrap items-center gap-2">
                  <Badge
                    className={gate.blocked ? 'bg-nfbg text-nf border-nfbd' : 'bg-ok-bg text-ok'}
                  >
                    {gate.blocked ? t.gateBlocked : t.gateClear}
                  </Badge>
                  {gate.blocked && (
                    <span className="text-muted-foreground text-[12.5px]">
                      {gate.blocking.agent_cases + gate.blocking.human_cases} {t.gateBlockedSub}
                    </span>
                  )}
                  {gate.advisory_outstanding > 0 && (
                    <span className="text-muted-foreground text-[12px]">
                      {gate.advisory_outstanding} {t.gateAdvisory}
                    </span>
                  )}
                </div>
              )}
              {worklist && (worklist.agent.cases > 0 || worklist.human.cases > 0) && (
                <div className="text-muted-foreground flex flex-wrap items-center gap-x-4 gap-y-1 text-[12.5px]">
                  <span>
                    <span className="text-foreground font-[700]">{worklist.agent.cases}</span>{' '}
                    {t.worklistCases} {t.worklistAgent}
                    {worklist.agent.minutes ? ` · ~${worklist.agent.minutes} ${t.minutes}` : ''}
                  </span>
                  <span>
                    <span className="text-foreground font-[700]">{worklist.human.cases}</span>{' '}
                    {t.worklistCases} {t.worklistHuman}
                    {worklist.human.minutes ? ` · ~${worklist.human.minutes} ${t.minutes}` : ''}
                  </span>
                </div>
              )}
            </Card>
          )}

          {/* What you are looking at, when it is not everything. A filter with
              no way back to the whole list is a dead end. */}
          {!compact && nodeFilter && (
            <div className="border-border-soft text-muted-foreground flex flex-wrap items-center gap-2 rounded-md border px-3 py-2 text-[12.5px]">
              <span>{t.filteredToSection}</span>
              <span className="text-foreground font-[650]">
                {nodeTitles.get(nodeFilter)?.title ?? nodeFilter}
              </span>
              <span className="grow" />
              {nodeTitles.get(nodeFilter) && (
                <button
                  type="button"
                  className="text-primary cursor-pointer font-[650]"
                  onClick={() => navigate(mapLink(nodeTitles.get(nodeFilter)!.mindmap, nodeFilter))}
                >
                  {t.openOnMap}
                </button>
              )}
              <button
                type="button"
                className="text-primary cursor-pointer font-[650]"
                onClick={() => setNodeFilter(null)}
              >
                {t.showAllChecks}
              </button>
            </div>
          )}

          {filtered.length === 0 ? (
            <div className="text-muted-foreground px-1 py-7.5 text-center text-[13.5px]">
              <p>{nodeFilter ? t.emptyForSection : t.empty}</p>
              <p className="mt-2 text-[12.5px] opacity-80">{t.emptyHint}</p>
            </div>
          ) : (
            ordered.map(([key, rows]) => (
              <div key={key} className="flex flex-col gap-2.5">
                <div className="flex flex-wrap items-baseline gap-2 px-0.5 pt-1.5">
                  <span className="text-muted-foreground text-[11.5px] font-[750] tracking-[0.06em] uppercase">
                    {key === UNASSIGNED ? t.unassigned : (initiativeTitles[key] ?? key)}
                  </span>
                  <span className="text-muted-foreground text-[11.5px]">{rows.length}</span>
                  {key === UNASSIGNED && (
                    <span className="text-muted-foreground text-[11.5px] opacity-70">
                      {t.unassignedHint}
                    </span>
                  )}
                  <span className="grow" />
                  {canWrite && key !== UNASSIGNED && (
                    <button
                      type="button"
                      className="text-primary cursor-pointer text-[12px] font-[650]"
                      onClick={() => {
                        setCreateUnder(key)
                        setCreating(true)
                      }}
                    >
                      + {t.newCheck}
                    </button>
                  )}
                </div>
                {rows.map((c) => (
                  <CheckCard
                    key={c.id}
                    check={c}
                    editLabel={t.liveEdit}
                    onEdit={() => editCheck(c.id)}
                    nodeTitle={c.node ? nodeTitles.get(c.node)?.title : undefined}
                    onOpenNode={
                      c.node && nodeTitles.get(c.node)
                        ? () => navigate(mapLink(nodeTitles.get(c.node!)!.mindmap, c.node))
                        : undefined
                    }
                    cases={cases[c.id]}
                    loadingCases={!!loadingCases[c.id]}
                    canWrite={canWrite}
                    canApprove={canApprove}
                    labels={cardLabels}
                    onToggleCases={() => void toggleCases(c.id)}
                    onVerdict={(caseId, verdict, opts) =>
                      void onVerdict(c.id, caseId, verdict, opts)
                    }
                    onArchive={() => {
                      if (!window.confirm(t.confirmArchiveCheck)) return
                      void archiveCheck(token, c.id)
                        .then(() => fetchAll())
                        .catch(handleErr)
                    }}
                  />
                ))}
              </div>
            ))
          )}
        </div>
      </main>

      <CheckDialog
        open={creating}
        onOpenChange={setCreating}
        initiatives={Object.entries(initiativeTitles).map(([id, title]) => ({ id, title }))}
        environments={environments.map((e) => ({ id: e.id, slug: e.slug }))}
        defaultInitiative={createUnder}
        nodes={planNodes.map((n) => ({ id: n.id, title: n.title }))}
        defaultNode={nodeFilter || undefined}
        labels={t}
        onSubmit={async (fields: CheckFields) => {
          await createCheck(token, project, fields)
          await fetchAll()
        }}
      />
    </>
  )
}
