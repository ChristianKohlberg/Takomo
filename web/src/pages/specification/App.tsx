import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Navigate, useLocation, useNavigate } from 'react-router'
import { AppShell } from '@/components/AppShell'
import { AppHeader } from '@/components/AppHeader'
import { ViewSwitcher } from '@/components/ViewSwitcher'
import { TokenGate } from '@/components/TokenGate'
import { SaveStatus } from '@/components/SaveStatus'
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
  mintMindmapSession,
  type Mindmap,
  type MindmapSession,
} from '@/lib/mindmaps'
import { openSpecification } from '@/lib/open-specification'
import { listChecks, type Check } from '@/lib/verification'
import { listDefinitions, type TestDefinition } from '@/lib/test-runs'
import { readPlanTree, nodesMap } from '@/lib/mindmap-crdt'
import { sameTree, type PlanNode } from '@/lib/plan-sections'
import { retryConnection } from '@/lib/retry-connection'
import type { SaveState } from '@/lib/save-status'
import { STR as DOCUMENT_STR } from '../documents/strings'
import { STR as CHECK_STR } from '../verification/strings'
import { SpecificationContext } from './context'
import { SpecificationViews } from './Views'

const History = lazy(() => import('./History'))

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
    tests: 'Section tests',
    choose: 'Choose a project to open its specification.',
  },
  de: {
    title: 'Spezifikation',
    untitled: 'Projektplan',
    loading: 'Spezifikation laden…',
    all: 'Gesamte Spezifikation',
    tests: 'Abschnittstests',
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
  const [section] = useWorkspaceSection()
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
  const [testDefinitions, setTestDefinitions] = useState<TestDefinition[]>([])
  const [saveState, setSaveState] = useState<SaveState>('connecting')
  const [peers, setPeers] = useState<string[]>([])
  const [nodes, setNodes] = useState<PlanNode[]>([])
  const access = useRef({ canWrite: false, title: project })
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
      ? await openSpecification(token, project, access.current.title, access.current.canWrite)
      : null
    if (epochs.current.map === epoch) {
      setMap(result)
      setLoaded(true)
    }
    return result
  }, [project, token])
  const refreshChecks = useCallback(async () => {
    const epoch = ++epochs.current.checks
    const [items, definitions] = project ? await Promise.all([
      listChecks(token, project).then(page => page.items), listDefinitions(token, project),
    ]) : [[], []]
    if (epochs.current.checks === epoch) { setChecks(items); setTestDefinitions(definitions) }
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
      access.current = { canWrite: (who.scopes ?? []).includes('write'), title: items.find((item) => item.id === project)?.name || project }
      await Promise.all([refreshMap(), refreshChecks()])
      if (!project && items[0]) navigate(specificationLink(items[0].id), { replace: true })
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
    for (const item of testDefinitions) {
      const check = item.definition
      if (!check.node) continue
      const count = result.get(check.node) ?? { total: 0, failing: 0 }
      count.total++
      if (item.execution.state === 'failed') count.failing++
      result.set(check.node, count)
    }
    return result
  }, [testDefinitions])
  const testsFor = useCallback((id: string) => counts.get(id) ?? { total: 0, failing: 0 }, [counts])
  const selected = nodes.find((node) => node.id === section)
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
      saveState,
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
      saveState,
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
    <main className="min-h-0 flex-1 bg-white p-5 dark:bg-card">
      {!project && <p className="text-sm text-muted-foreground">{w.choose}</p>}
      {project && !access.current.canWrite && <p className="text-sm text-muted-foreground">{t.readOnlyBanner}</p>}
    </main>
  )
  return (
    <SpecificationContext value={context}>
      <ProjectUpdatesContext value={updates}>
        <AppShell
          lang={lang}
          onLang={(value) => {
            setLang(value)
            localStorage.setItem('takomo.lang', value)
          }}
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
            {map && <Suspense fallback={null}><History /></Suspense>}
            {peers.length > 0 && (
              <span
                className="max-w-60 truncate text-xs text-muted-foreground"
                title={peers.join(', ')}
              >
                {peers.join(', ')}
              </span>
            )}
          </AppHeader>
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
        </AppShell>
      </ProjectUpdatesContext>
    </SpecificationContext>
  )
}

export default App
