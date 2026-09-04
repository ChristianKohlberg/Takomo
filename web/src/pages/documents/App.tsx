// /documents — the project's plan, written out.
//
// A project has ONE plan. The map and this page are two renderings of it, not
// two things kept in step: a node is a section, its title is the heading, its
// depth is the heading level, and tree order is reading order
// (`spec/one-model-two-views.md`). The canvas is for growing and grouping
// thoughts fast; this is where they are spelled out, and where the history of
// who wrote and who agreed is visible.
//
// What used to be here was a FILE BROWSER over `documents` rows, filled by
// converting a map into a tree of them. That conversion is gone, and the reason
// it had to go is worth keeping written down: a node's notes and its document's
// prose were two places one paragraph lived, and they disagreed after the first
// edit. Linking two copies is not the same as having one thing.
//
// So this page owns exactly three things: the map's socket ticket, the plan's
// history (which is SQL, not CRDT — see `src/store/trace.rs`), and the shell
// around them. `Plan` owns the document.
import { ViewSwitcher } from '@/components'
import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Hint } from '@/components/Hint'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { whoami, listProjects, type Project } from '@/lib/initiatives'
import {
  getMindmap,
  getTrace,
  listMindmaps,
  mintMindmapSession,
  recordTrace,
  type Mindmap,
  type MindmapSession,
  type PlanStanding,
  type TraceEntry,
} from '@/lib/mindmaps'
import { traceByNode } from '@/lib/plan-trace'
import { mapLink, readPlanFocus } from '@/lib/plan-url'
import { STR } from './strings'
import type { ConnectionState } from './Plan'

// Tiptap, ProseMirror, Yjs and the socket together outweigh the rest of the app,
// and every other surface would pay for them on first paint. This must stay a
// lazy import.
const Plan = lazy(() => import('./Plan'))

const LS_LANG = 'takomo.lang'

/** How much of the plan's history one read brings back. The server's ceiling is
 *  500; a plan is capped at 500 sections, so this is a page, not everything. */
const TRACE_LIMIT = 500

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => loadProject())
  const [gateError, setGateError] = useState('')

  const [actor, setActor] = useState('')
  const [scopes, setScopes] = useState<string[]>([])
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [projects, setProjects] = useState<Project[]>([])

  const [map, setMap] = useState<Mindmap | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [session, setSession] = useState<MindmapSession | null>(null)
  const [connection, setConnection] = useState<ConnectionState>('connecting')
  const [peers, setPeers] = useState<string[]>([])
  /**
   * `#n=<node>`, the section the map asked to be shown.
   *
   * Read once, at mount, and cleared the moment the plan honours it: it is a
   * hand-off rather than state the two views keep in step, exactly as `#n=` is
   * in the other direction.
   */
  const [focusSection, setFocusSection] = useState<string | null>(() =>
    readPlanFocus(window.location.hash),
  )
  const [standing, setStanding] = useState<PlanStanding>({})
  const [entries, setEntries] = useState<TraceEntry[]>([])

  const t = useMemo(() => pick(STR, lang), [lang])

  function signOut() {
    saveToken('')
    setToken('')
    setMap(null)
    setSession(null)
  }

  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { message?: string }
      if (isAuthError(e)) {
        saveToken('')
        setToken('')
        setGateError('')
        return
      }
      toast(err?.message || t.requestFailed, 'err')
    },
    [toast, t],
  )

  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const who = await whoami(token)
        if (cancelled) return
        const sc = who.scopes ?? []
        if (!sc.includes('read')) {
          saveToken('')
          setToken('')
          setGateError(t.gateNoRead)
          return
        }
        setActor(who.actor ?? '')
        setScopes(sc)
        setProjects(await listProjects(token).catch(() => []))
      } catch (e) {
        if (!cancelled) handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, handleErr, t])

  // A project holds exactly one plan, so this is a lookup rather than a list.
  const findMap = useCallback(async () => {
    if (!project) {
      setMap(null)
      setLoaded(true)
      return null
    }
    const page = await listMindmaps(token, { project, limit: 1 })
    const found = page.items[0] ?? null
    setMap(found)
    setLoaded(true)
    return found
  }, [token, project])

  useEffect(() => {
    if (!token) return
    setLoaded(false)
    setSession(null)
    findMap().catch(handleErr)
  }, [token, findMap, handleErr])

  /**
   * The plan's history, in two reads that go together.
   *
   * Standing rides on the map's own detail read because the server computes it
   * there in one grouped query — a section confirmed BEFORE its last edit is not
   * confirmed any more, which no stored flag can say. The trace is the acts
   * behind that reading.
   */
  const mapId = map?.id ?? null
  const refreshHistory = useCallback(async () => {
    if (!mapId) return
    const [detail, page] = await Promise.all([
      getMindmap(token, mapId),
      getTrace(token, mapId, { limit: TRACE_LIMIT }),
    ])
    setStanding(detail.standing ?? {})
    setEntries(page.items)
  }, [token, mapId])

  useEffect(() => {
    if (!token || !mapId) return
    refreshHistory().catch(handleErr)
  }, [token, mapId, refreshHistory, handleErr])

  // Opening the plan means minting a ticket for the map's socket. Done here so
  // the view never has to know about tokens — it receives a session and connects.
  useEffect(() => {
    if (!token || !mapId) {
      setSession(null)
      return
    }
    let cancelled = false
    setSession(null)
    setConnection('connecting')
    setPeers([])
    mintMindmapSession(token, mapId)
      .then((s) => {
        if (!cancelled) setSession(s)
      })
      .catch((e) => {
        if (!cancelled) handleErr(e)
      })
    return () => {
      cancelled = true
    }
  }, [token, mapId, handleErr])

  const onConnection = useCallback((s: ConnectionState) => setConnection(s), [])
  const onPeers = useCallback((names: string[]) => setPeers(names), [])

  /**
   * Bring the history back in line, once, after a burst of writes.
   *
   * Somebody typing in three sections files three `edited` entries, and each of
   * them would otherwise refetch the whole plan's history. The trace is sparse
   * by contract; the reads of it should be too.
   */
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const scheduleRefresh = useCallback(() => {
    if (pending.current) clearTimeout(pending.current)
    pending.current = setTimeout(() => {
      pending.current = null
      refreshHistory().catch(handleErr)
    }, 1500)
  }, [refreshHistory, handleErr])

  useEffect(
    () => () => {
      if (pending.current) clearTimeout(pending.current)
    },
    [],
  )

  const onReview = useCallback(
    (node: string) => {
      if (!mapId) return
      recordTrace(token, mapId, { kind: 'reviewed', node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  const onEdited = useCallback(
    (node: string) => {
      if (!mapId) return
      recordTrace(token, mapId, { kind: 'edited', node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  /** The map is where a section is named, moved and pruned. This hands it over
   *  by link, and the canvas selects and centres what arrives. */
  const onShowOnMap = useCallback(
    (node: string) => {
      if (!mapId) return
      navigate(mapLink(mapId, node))
    },
    [navigate, mapId],
  )

  const onFocusedSection = useCallback(() => setFocusSection(null), [])

  /**
   * A decision on an agent's proposal, filed as an act on the section.
   *
   * The plan applies the change itself — the browser is the only place that can,
   * because markdown becomes nodes in the editor's own schema — so the server
   * never sees the edit as a request. What it gets is the record of who decided
   * what, which is the half that has to survive compaction.
   */
  const onDecided = useCallback(
    (node: string, kind: 'accepted' | 'rejected') => {
      if (!mapId) return
      recordTrace(token, mapId, { kind, node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  /** Ops an accepted proposal could not apply, said out loud. A reviewer who
   *  believes they accepted the whole change has not reviewed it. */
  const onSkipped = useCallback(
    (messages: string[]) => {
      toast(`${t.applySkipped.replace('{n}', String(messages.length))} ${messages.join('; ')}`, 'err')
    },
    [toast, t],
  )

  const trace = useMemo(() => traceByNode(entries), [entries])

  if (!token) {
    return (
      <TokenGate
        title="takomo · documents"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.tokenNeeded}
        error={gateError}
        onSubmit={(tk) => {
          saveToken(tk)
          setGateError('')
          setToken(tk)
        }}
      />
    )
  }

  const connectionLabel =
    connection === 'connected'
      ? t.connected
      : connection === 'connecting'
        ? t.connecting
        : t.disconnected

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'specification',
        nav: {
          board: t.board,
          inbox: t.inbox,
          specification: t.specification,
          initiatives: t.initiatives,
          schedules: t.schedules,
          environments: t.environments,
        },
        projects: projects.map(({ id, name, archived, archived_at }) => ({
          id,
          name,
          archived,
          archived_at,
        })),
        project,
        onProject: (id) => {
          setProject(id)
          saveProject(id)
        },
        projectLabels: {
          project: t.project,
          search: t.projectSearch,
          noMatch: t.projectNoMatch,
          all: t.allProjects,
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
        onSignOut: signOut,
      }}
    >
      <AppHeader
        title={map ? map.title : t.documents}
        views={
          <ViewSwitcher
            current="document"
            onNavigate={navigate}
            labels={{ map: t.viewMap, document: t.viewDocument, tests: t.viewTests }}
          />
        }
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        {session && (
          <>
            <span
              className={
                'text-[11.5px] font-bold ' +
                (connection === 'connected'
                  ? 'text-emerald-600 dark:text-emerald-400'
                  : 'text-muted-foreground')
              }
            >
              ● {connectionLabel}
            </span>
            <span className="text-muted-foreground text-[11.5px]">
              {peers.length ? `${t.alsoHere}: ${peers.join(', ')}` : t.justYou}
            </span>
          </>
        )}
        <Hint text={t.refresh}>
          <Button
            variant="outline"
            size="icon"
            onClick={() => {
              findMap().catch(handleErr)
              refreshHistory().catch(handleErr)
            }}
          >
            ↻
          </Button>
        </Hint>
      </AppHeader>

      {!project ? (
        <p className="text-muted-foreground px-4 py-8 text-[13.5px]">{t.pickProject}</p>
      ) : loaded && !map ? (
        <div className="text-muted-foreground px-4 py-8 text-[13.5px]">
          <p>{t.noPlan}</p>
          <p className="mt-2 opacity-80">{t.noPlanHint}</p>
          <Button className="mt-3" variant="outline" onClick={() => navigate('/mindmaps')}>
            {t.openMap}
          </Button>
        </div>
      ) : session ? (
        <Suspense
          fallback={<p className="text-muted-foreground px-4 py-8 text-[13px]">{t.connecting}</p>}
        >
          <Plan
            key={session.session}
            session={session}
            standing={standing}
            trace={trace}
            onConnection={onConnection}
            onPeers={onPeers}
            onReview={onReview}
            onEdited={onEdited}
            onShowOnMap={onShowOnMap}
            onDecided={onDecided}
            onSkipped={onSkipped}
            focusSection={focusSection}
            onFocusedSection={onFocusedSection}
            labels={{
              readOnly: t.readOnlyBanner,
              empty: t.empty,
              emptyHint: t.emptyHint,
              proseEmpty: t.proseEmpty,
              proseLabel: t.proseLabel,
            }}
            railLabels={{
              outline: t.outline,
              expand: t.outlineExpand,
              collapse: t.outlineCollapse,
              folded: t.outlineFolded,
              untitled: t.untitled,
              standingConfirmed: t.standingConfirmed,
              standingChanged: t.standingChanged,
              standingUnseen: t.standingUnseen,
              pending: t.railPending,
            }}
            sectionLabels={{
              untitled: t.untitled,
              standingConfirmed: t.standingConfirmed,
              standingChanged: t.standingChanged,
              standingUnseen: t.standingUnseen,
              review: t.review,
              reviewHint: t.reviewHint,
              showOnMap: t.showOnMap,
              history: t.history,
              hideHistory: t.hideHistory,
              historyEmpty: t.historyEmpty,
              historyMore: t.historyMore,
              proposals: t.proposals,
              hideProposals: t.hideProposals,
              pendingBadge: t.pendingBadge,
              needWrite: t.needWrite,
              kinds: {
                authored: t.kindAuthored,
                renamed: t.kindRenamed,
                edited: t.kindEdited,
                moved: t.kindMoved,
                pruned: t.kindPruned,
                reviewed: t.kindReviewed,
                proposed: t.kindProposed,
                accepted: t.kindAccepted,
                rejected: t.kindRejected,
              },
            }}
            proposalLabels={{
              heading: t.proposalsHeading,
              empty: t.proposalsEmpty,
              pending: t.proposalPending,
              accepted: t.proposalAccepted,
              rejected: t.proposalRejected,
              accept: t.accept,
              reject: t.reject,
              by: t.proposalBy,
              partial: t.proposalPartial,
              opReplace: t.opReplace,
              opInsert: t.opInsert,
              opDelete: t.opDelete,
              readOnly: t.proposalReadOnly,
            }}
          />
        </Suspense>
      ) : (
        <p className="text-muted-foreground px-4 py-8 text-[13px]">{t.connecting}</p>
      )}
    </AppShell>
  )
}
