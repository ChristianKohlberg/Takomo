import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router'
import { AppShell } from '@/components/AppShell'
import { AppHeader } from '@/components/AppHeader'
import { ViewSwitcher } from '@/components/ViewSwitcher'
import { TokenGate } from '@/components/TokenGate'
import { SaveStatus } from '@/components/SaveStatus'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from '@/components/ui/sheet'
import { CheckEditor } from '@/components/verification/CheckEditor'
import { useToast } from '@/components/Toaster'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useSyncConnection } from '@/hooks/useSyncConnection'
import { ProjectUpdatesContext, useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceProject } from '@/hooks/useWorkspace'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { specificationLink, specificationProject, specificationView } from '@/lib/specification-url'
import { loadToken, saveToken, isAuthError } from '@/lib/session'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import {
  createMindmap,
  listMindmaps,
  mintMindmapSession,
  type Mindmap,
  type MindmapSession,
} from '@/lib/mindmaps'
import { listChecks, worstState, type Check } from '@/lib/verification'
import { readPlanTree, nodesMap } from '@/lib/mindmap-crdt'
import { sameTree, type PlanNode } from '@/lib/plan-sections'
import { retryConnection } from '@/lib/retry-connection'
import type { SaveState } from '@/lib/save-status'
import { STR as DOCUMENT_STR } from '../documents/strings'
import { STR as CHECK_STR } from '../verification/strings'
import { SpecificationContext } from './context'
import { SpecificationViews } from './Views'

const DocumentView = lazy(() =>
  import('../documents/App').then((m) => ({ default: m.DocumentView })),
)
const MapView = lazy(() => import('../mindmap/App').then((m) => ({ default: m.MapView })))
const TestsView = lazy(() => import('../verification/App').then((m) => ({ default: m.TestsView })))
const words = {
  en: {
    title: 'Specification',
    untitled: 'Project plan',
    loading: 'Loading specification…',
    all: 'Whole specification',
    missing: 'This section is no longer available',
    tests: 'Section tests',
    failed: 'failing',
    create: 'Create plan',
    empty: 'One plan, three views',
    hint: 'Start your project’s specification. Its document and map share the same sections; tests verify those sections.',
    name: 'Plan title',
    cancel: 'Cancel',
    close: 'Close tests',
    noTests: 'No linked tests yet',
    choose: 'Choose a project to open its specification.',
  },
  de: {
    title: 'Spezifikation',
    untitled: 'Projektplan',
    loading: 'Spezifikation laden…',
    all: 'Gesamte Spezifikation',
    missing: 'Dieser Abschnitt ist nicht mehr verfügbar',
    tests: 'Abschnittstests',
    failed: 'fehlgeschlagen',
    create: 'Plan erstellen',
    empty: 'Ein Plan, drei Ansichten',
    hint: 'Beginne die Spezifikation deines Projekts. Dokument und Map teilen dieselben Abschnitte; Tests prüfen diese Abschnitte.',
    name: 'Plantitel',
    cancel: 'Abbrechen',
    close: 'Tests schließen',
    noTests: 'Noch keine verknüpften Tests',
    choose: 'Wähle ein Projekt, um seine Spezifikation zu öffnen.',
  },
}

export function App() {
  const [project, selectProject] = useWorkspaceProject()
  const location = useLocation()
  if (!specificationProject(location.pathname) && project)
    return (
      <Navigate
        to={{ pathname: specificationLink(project).split('?')[0], search: location.search }}
        replace
      />
    )
  return <SpecificationWorkspace key={project} project={project} selectProject={selectProject} />
}

function SpecificationWorkspace({
  project,
  selectProject,
}: {
  project: string
  selectProject: (id: string) => void
}) {
  const navigate = useNavigate()
  const location = useLocation()
  const view = specificationView(location.search)
  const query = new URLSearchParams(location.search)
  const [section, selectSection] = useWorkspaceSection()
  const panel = query.get('panel') === 'tests' && view !== 'tests'
  const editing = query.get('check')
  const [token, setToken] = useState(loadToken)
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem('takomo.lang')))
  const [actor, setActor] = useState('')
  const [scopes, setScopes] = useState<string[]>([])
  const [voice, setVoice] = useState(false)
  const [projects, setProjects] = useState<Project[]>([])
  const [map, setMap] = useState<Mindmap | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [session, setSession] = useState<MindmapSession | null>(null)
  const [checks, setChecks] = useState<Check[]>([])
  const [saveState, setSaveState] = useState<SaveState>('connecting')
  const [peers, setPeers] = useState<string[]>([])
  const [nodes, setNodes] = useState<PlanNode[]>([])
  const [creating, setCreating] = useState(false)
  const [title, setTitle] = useState('')
  const [busy, setBusy] = useState(false)
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const { toast } = useToast()
  const t = pick(DOCUMENT_STR, lang)
  const c = pick(CHECK_STR, lang)
  const w = words[lang]
  const onError = useCallback(
    (error: unknown) => {
      if (isAuthError(error)) {
        saveToken('')
        setToken('')
        return
      }
      toast(error instanceof Error ? error.message : String(error), 'err')
    },
    [toast],
  )
  const epochs = useRef({ map: 0, checks: 0 })
  useEffect(() => {
    const ref = epochs
    return () => {
      ref.current.map++
      ref.current.checks++
    }
  }, [])
  const refreshMap = useCallback(async () => {
    const epoch = ++epochs.current.map
    const result = project
      ? ((await listMindmaps(token, { project, limit: 1 })).items[0] ?? null)
      : null
    if (epochs.current.map === epoch) {
      setMap(result)
      setLoaded(true)
    }
    return result
  }, [project, token])
  const refreshChecks = useCallback(async () => {
    const epoch = ++epochs.current.checks
    const items = project ? (await listChecks(token, project)).items : []
    if (epochs.current.checks === epoch) setChecks(items)
    return items
  }, [project, token])
  useEffect(() => {
    if (!token) return
    let cancelled = false
    const abort = new AbortController()
    void retryConnection(async () => {
      const [who, items] = await Promise.all([whoami(token), listProjects(token)])
      if (cancelled) return
      setActor(who.actor ?? '')
      setScopes(who.scopes ?? [])
      setVoice(who.features?.voice === true)
      setProjects(items)
      if (!project && items[0]) navigate(specificationLink(items[0].id), { replace: true })
    }, abort.signal).catch((error) => {
      if (!cancelled) onError(error)
    })
    void retryConnection(async () => {
      await Promise.all([refreshMap(), refreshChecks()])
    }, abort.signal).catch((error) => {
      if (!cancelled) onError(error)
    })
    return () => {
      cancelled = true
      abort.abort()
    }
  }, [token, project, navigate, onError, refreshMap, refreshChecks])
  const mapId = map?.id
  useEffect(() => {
    setSession(null)
    if (!token || !mapId) return
    let cancelled = false
    const abort = new AbortController()
    void retryConnection(() => mintMindmapSession(token, mapId), abort.signal)
      .then((value) => {
        if (!cancelled) setSession(value)
      })
      .catch((error) => {
        if (!cancelled) onError(error)
      })
    return () => {
      cancelled = true
      abort.abort()
    }
  }, [token, mapId, onError])
  const connection = useSyncConnection(session, onError, setSaveState)
  useEffect(() => {
    if (!connection || !session) {
      setNodes([])
      setPeers([])
      return
    }
    const { ydoc, provider } = connection
    const read = () => {
      const next = readPlanTree(ydoc)
      setNodes((current) => (sameTree(current, next) ? current : next))
    }
    const awareness = () => {
      const names: string[] = []
      provider.awareness.getStates().forEach((value, id) => {
        if (id !== provider.awareness.clientID && typeof value.user?.name === 'string')
          names.push(value.user.name)
      })
      setPeers(names)
    }
    read()
    awareness()
    nodesMap(ydoc).observeDeep(read)
    provider.awareness.on('change', awareness)
    provider.awareness.setLocalStateField('user', { name: session.display, color: '#1f4e78' })
    provider.connect()
    return () => {
      nodesMap(ydoc).unobserveDeep(read)
      provider.awareness.off('change', awareness)
    }
  }, [connection, session])
  useEffect(() => {
    connection?.provider.awareness.setLocalStateField('mm', { selected: section })
  }, [connection, section])
  const listeners = useRef(new Set<() => Promise<unknown>>())
  const updates = useMemo(
    () => ({
      project,
      subscribe: (callback: () => Promise<unknown>) => {
        listeners.current.add(callback)
        return () => {
          listeners.current.delete(callback)
        }
      },
    }),
    [project],
  )
  useProjectUpdates(token, project, async () => {
    await Promise.allSettled([refreshMap(), refreshChecks()])
    await Promise.allSettled([...listeners.current].map((callback) => callback()))
  })
  const changeQuery = useCallback(
    (changes: Record<string, string | null>) => {
      const search = new URLSearchParams(location.search)
      for (const [key, value] of Object.entries(changes)) {
        if (value === null) search.delete(key)
        else search.set(key, value)
      }
      navigate({ search: search.toString() }, { replace: true })
    },
    [location.search, navigate],
  )
  const openTests = useCallback(
    (id: string | null) => changeQuery({ section: id, panel: 'tests', check: null }),
    [changeQuery],
  )
  const editCheck = useCallback((id: string) => changeQuery({ check: id }), [changeQuery])
  const counts = useMemo(() => {
    const result = new Map<string, { total: number; failing: number }>()
    for (const check of checks) {
      if (!check.node || check.archived_at) continue
      const count = result.get(check.node) ?? { total: 0, failing: 0 }
      count.total++
      if (worstState(check.cases) === 'failed') count.failing++
      result.set(check.node, count)
    }
    return result
  }, [checks])
  const testsFor = useCallback((id: string) => counts.get(id) ?? { total: 0, failing: 0 }, [counts])
  const selected = nodes.find((node) => node.id === section)
  const breadcrumbs = useMemo(() => {
    const path: PlanNode[] = []
    const seen = new Set<string>()
    let node = nodes.find((n) => n.id === section)
    while (node && !seen.has(node.id)) {
      seen.add(node.id)
      path.unshift(node)
      const parent = node.parent
      node = nodes.find((n) => n.id === parent)
    }
    return path
  }, [nodes, section])
  const context = useMemo(
    () => ({
      token,
      lang,
      project,
      projects,
      actor,
      scopes,
      voice,
      map,
      session,
      connection,
      checks,
      setChecks,
      refreshMap,
      refreshChecks,
      selectProject,
      onError,
      openTests,
      editCheck,
      testsFor,
      nodes,
    }),
    [
      token,
      lang,
      project,
      projects,
      actor,
      scopes,
      voice,
      map,
      session,
      connection,
      checks,
      refreshMap,
      refreshChecks,
      selectProject,
      onError,
      openTests,
      editCheck,
      testsFor,
      nodes,
    ],
  )
  if (!token)
    return (
      <TokenGate
        title="takomo · specification"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.tokenNeeded}
        error=""
        onSubmit={(value) => {
          saveToken(value)
          setToken(value)
        }}
      />
    )
  const empty = (
    <div className="mx-auto flex max-w-lg flex-col items-start gap-3 p-6">
      <h2 className="text-lg font-semibold">{w.empty}</h2>
      <p className="text-sm text-muted-foreground">{project ? w.hint : w.choose}</p>
      {project && scopes.includes('write') && (
        <Button onClick={() => setCreating(true)}>{w.create}</Button>
      )}
    </div>
  )
  return (
    <SpecificationContext value={context}>
      <ProjectUpdatesContext value={updates}>
        <AppShell
          rail={{
            current: 'specification',
            nav: {
              board: t.board,
              inbox: t.inbox,
              specification: w.title,
              initiatives: t.initiatives,
              schedules: t.schedules,
              environments: t.environments,
            },
            projects,
            project,
            onProject: selectProject,
            projectLabels: {
              project: t.project,
              search: t.projectSearch,
              noMatch: t.projectNoMatch,
            },
            labels: {
              expand: t.navExpand,
              collapse: t.navCollapse,
              signOut: t.signOut,
              account: t.navAccount,
              settings: t.settings,
            },
            collapsed: navCollapsed,
            onCollapsed: setNavCollapsed,
            actor,
            scopes,
            onNavigate: navigate,
            onSignOut: () => {
              saveToken('')
              setToken('')
            },
          }}
        >
          <AppHeader
            title={w.title}
            subtitle={map?.title ?? w.untitled}
            lang={lang}
            onLang={(value) => {
              setLang(value)
              localStorage.setItem('takomo.lang', value)
            }}
            views={
              <ViewSwitcher
                current={view}
                labels={{ document: t.viewDocument, map: t.viewMap, tests: t.viewTests }}
                onNavigate={navigate}
              />
            }
          >
            {session && <SaveStatus state={saveState} lang={lang} />}
            {peers.length > 0 && (
              <span
                className="max-w-60 truncate text-xs text-muted-foreground"
                title={peers.join(', ')}
              >
                {peers.join(', ')}
              </span>
            )}
          </AppHeader>
          <div className="flex min-w-0 flex-none flex-wrap items-center justify-between gap-2 border-b px-3 py-2 text-xs sm:px-5">
            <nav aria-label={w.title} className="flex min-w-0 flex-wrap items-center gap-1">
              <button
                className="cursor-pointer text-muted-foreground hover:text-foreground"
                onClick={() => selectSection(null)}
              >
                {w.all}
              </button>
              {breadcrumbs.map((node) => (
                <span key={node.id} className="flex min-w-0 items-center gap-1">
                  <span aria-hidden>/</span>
                  <button
                    className="max-w-60 cursor-pointer truncate"
                    onClick={() => selectSection(node.id)}
                  >
                    {node.title}
                  </button>
                </span>
              ))}
              {section && !selected && <span className="text-muted-foreground">{w.missing}</span>}
            </nav>
            {section && view !== 'tests' && (
              <Button size="sm" variant="outline" onClick={() => openTests(section)}>
                {t.viewTests} ({testsFor(section).total})
                {testsFor(section).failing > 0 && (
                  <span className="text-destructive">
                    {' '}
                    · {testsFor(section).failing} {w.failed}
                  </span>
                )}
              </Button>
            )}
          </div>
          <SpecificationViews
            current={view}
            loading={w.loading}
            views={{
              document: !loaded ? (
                <p className="p-5">{w.loading}</p>
              ) : !map ? (
                empty
              ) : connection && session ? (
                <DocumentView />
              ) : (
                <p role="status" className="p-5">
                  {w.loading}
                </p>
              ),
              map: !loaded ? (
                <p className="p-5">{w.loading}</p>
              ) : !map ? (
                empty
              ) : connection && session ? (
                <MapView />
              ) : (
                <p role="status" className="p-5">
                  {w.loading}
                </p>
              ),
              tests: <TestsView />,
            }}
          />
          <Sheet
            open={panel}
            onOpenChange={(open) => {
              if (!open) changeQuery({ panel: null })
            }}
          >
            <SheetContent className="w-full gap-0 p-0 sm:max-w-xl">
              <SheetHeader className="border-b p-4">
                <SheetTitle>{w.tests}</SheetTitle>
                <SheetDescription>{selected?.title ?? w.all}</SheetDescription>
              </SheetHeader>
              {panel && (
                <Suspense fallback={<p className="p-4">{w.loading}</p>}>
                  <TestsView compact />
                </Suspense>
              )}
            </SheetContent>
          </Sheet>
          {editing && checks.some((check) => check.id === editing) && (
            <CheckEditor
              lang={lang}
              token={token}
              id={editing}
              labels={c}
              onError={onError}
              onClose={() => changeQuery({ check: null })}
            />
          )}
          <Dialog open={creating} onOpenChange={setCreating}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{w.create}</DialogTitle>
                <DialogDescription>{w.hint}</DialogDescription>
              </DialogHeader>
              <form
                className="grid gap-4"
                onSubmit={(event) => {
                  event.preventDefault()
                  if (!title.trim() || busy) return
                  setBusy(true)
                  void createMindmap(token, { project, title: title.trim() })
                    .then(async () => {
                      await refreshMap()
                      setCreating(false)
                      setTitle('')
                    })
                    .catch(onError)
                    .finally(() => setBusy(false))
                }}
              >
                <label className="grid gap-2 text-sm">
                  {w.name}
                  <Input
                    autoFocus
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                    maxLength={300}
                    required
                  />
                </label>
                <div className="flex justify-end gap-2">
                  <Button type="button" variant="outline" onClick={() => setCreating(false)}>
                    {w.cancel}
                  </Button>
                  <Button disabled={busy || !title.trim()}>{w.create}</Button>
                </div>
              </form>
            </DialogContent>
          </Dialog>
        </AppShell>
      </ProjectUpdatesContext>
    </SpecificationContext>
  )
}

export default App
