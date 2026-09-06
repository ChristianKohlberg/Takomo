// /board — one route, three audiences.
//
//   #a=tka_…  an outside expert answering ONE question (AnswerGrantPage)
//   #s=tks_…  a read-only share of a project or subtree (SharePage)
//   neither   the board itself, on a `tk_` token from localStorage
//
// The fragment wins over a stored token — see lib/board-mode.ts for why.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { CreateEpicDialog } from '@/components/board/CreateEpicDialog'
import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { useNavigate } from 'react-router'
import { useIsPhone } from '@/hooks/useIsPhone'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { Typeahead } from '@/components/Typeahead'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Column } from '@/components/board/Column'
import { EPICS_STR } from '@/components/board/epics-strings'
import { EpicsView } from '@/components/board/EpicsView'
import { AskDrawer } from '@/components/board/AskDrawer'
import { DetailPanel } from '@/components/board/DetailPanel'
import { InboxDrawer } from '@/components/board/InboxDrawer'
import { AnswerGrantPage } from './AnswerGrantPage'
import { SharePage } from './SharePage'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { modeFor } from '@/lib/board-mode'
import { countWaiting, listInitiatives, listProjects, whoami, type Project } from '@/lib/initiatives'
import { epicOf, inSubtree, indexById, matchesTagRefs } from '@/lib/tickets'
import { fetchRoadmap, laneTitles, type Roadmap } from '@/lib/roadmap'
import { listUsers } from '@/lib/users'
import { cn } from '@/lib/utils'
import {
  getEvents,
  getTicket,
  getWorkflow,
  hasEvents,
  listTickets,
  type Ticket,
  type Workflow,
} from '@/lib/board'
import { answerQuestion, askQuestion, listQuestions, type Question } from '@/lib/questions'
import { STR } from './strings'
import { Checkbox } from '@/components/ui/checkbox'
import { Hint } from '@/components/Hint'
import { Picker } from '@/components/Picker'

const LS_LANG = 'takomo.lang'
const POLL_MS = 4000

export function App({ surface = 'board' }: { surface?: 'board' | 'epics' }) {
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const t = useMemo(() => pick(STR, lang), [lang])

  // Read once: a grant is what the URL asked for at load, and re-reading it on
  // every render would fight the board's own hash writes.
  const [mode] = useState(() => modeFor(surface === 'board' ? window.location.hash : ''))

  if (mode.kind === 'answer') {
    return (
      <AnswerGrantPage
        token={mode.token}
        lang={lang}
        labels={{
          yes: t.approve,
          no: t.reject,
          writeOwn: t.customDivider,
          ownPlaceholder: t.customPlaceholder,
          textPlaceholder: t.answerPlaceholder,
          recommends: t.recommends,
          submit: t.submit,
          typeFirst: t.typeFirst,
          sendFirst: t.sendFirst,
          ticketCtx: t.ticketCtx,
          validUntil: t.validUntil,
          thanks: t.grantThanks,
          spent: t.grantSpent,
          expired: t.grantExpired,
        }}
      />
    )
  }

  if (mode.kind === 'share') {
    return (
      <SharePage
        token={mode.token}
        lang={lang}
        labels={{
          readOnly: t.shareRO,
          validUntil: t.validUntil,
          expired: t.shareExpired,
          showMore: t.showMore,
          blocked: t.blockedN,
          empty: t.shareEmpty,
          fromSchedule: t.fromSchedule,
          notFulfilled: t.notFulfilled,
        }}
      />
    )
  }

  return <Board surface={surface} lang={lang} setLang={setLang} deepTicket={mode.ticket} />
}

function Board({
  lang,
  setLang,
  deepTicket,
  surface,
}: {
  surface: 'board' | 'epics'
  lang: Locale
  setLang: (l: Locale) => void
  deepTicket?: string
}) {
  const navigate = useNavigate()
  const isPhone = useIsPhone()
  const { toast } = useToast()
  const t = useMemo(() => pick(STR, lang), [lang])

  const [token, setToken] = useState(() => loadToken())
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [project, setProject] = useState(() => loadProject())
  const [projects, setProjects] = useState<Project[]>([])
  const [projectsStatus, setProjectsStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [projectRetry, setProjectRetry] = useState(0)

  // The project selection is shared across all four surfaces, and `''` there
  // means ALL PROJECTS — a real state the inbox, initiatives and schedules each
  // offer. A kanban cannot show it: columns come from a project's workflow, and
  // two projects need not agree on their states.
  //
  // So the board NARROWS to a concrete project for its own rendering, and
  // deliberately does not write that back. Writing it back would mean a visit to
  // the board silently converted someone's "All projects" inbox into a
  // single-project one.
  const effectiveProject = project || projects[0]?.id || ''
  /**
   * People this project can address a question to, for the ask drawer. Empty on an
   * instance with no directory, which hides the control and leaves asking exactly
   * as it was.
   */
  const [askPeople, setAskPeople] = useState<{ handle: string; label: string }[]>([])
  const [workflow, setWorkflow] = useState<Workflow | null>(null)
  const [loadError, setLoadError] = useState(false)
  const [tickets, setTickets] = useState<Ticket[]>([])
  const [cursor, setCursor] = useState<number | string>(0)
  const [selectedId, setSelectedId] = useState<string | null>(deepTicket ?? null)

  const [filtersOpen, setFiltersOpen] = useState(false)
  // Which single column a phone is looking at.
  //
  // A kanban is horizontal by nature, and snap-scrolling eight columns through a
  // 375px window is a coping mechanism, not a design: you see one of eight and
  // have to swipe blind to find the rest. On a phone this picks ONE state and
  // gives it the full width; `md` and up still get the real board. `null` means
  // "not chosen yet" and resolves to the first state once the workflow loads.
  const [mobileState, setMobileState] = useState<string | null>(null)
  const [ticketFilter, setTicketFilter] = useState(deepTicket ?? '')
  const [tagKind, setTagKind] = useState('')
  const [tagFilter, setTagFilter] = useState('')
  const [epicFilter, setEpicFilter] = useState('')
  const [labelFilter, setLabelFilter] = useState('')
  const [groupByEpic, setGroupByEpic] = useState(false)
  // Which altitude the reader is at. `epics` is NOT the board grouped by epic —
  // that stays a ticket board and answers where each ticket is. This answers
  // where each epic is, who holds it, and whether it is moving.
  const view = surface
  const [creatingEpic, setCreatingEpic] = useState(false)
  const [roadmapStatus, setRoadmapStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const activeScope = useRef(effectiveProject)
  activeScope.current = effectiveProject
  const detailRequest = useRef(0)
  const detailContext = useRef({ token, project: effectiveProject })
  detailContext.current = { token, project: effectiveProject }
  useEffect(() => () => { detailRequest.current++ }, [token, effectiveProject])
  const [roadmapResult, setRoadmapResult] = useState<{ project: string; data: Roadmap }>()
  const roadmap = roadmapResult?.project === effectiveProject ? roadmapResult.data : undefined
  const availableRoadmap = useRef(roadmap)
  availableRoadmap.current = roadmap
  // Bumped when the event poll actually finds something, so the epics view
  // refreshes on real change rather than on every four-second tick — it is one
  // query per epic and does not belong on a timer.
  const [epoch, setEpoch] = useState(0)
  const [showArchived, setShowArchived] = useState(false)
  const [mineOnly, setMineOnly] = useState(false)

  // Six filters compose, each individually clearable — but with no count and no
  // way to clear them together, an empty board gave the reader no clue which of
  // the six did it or how many were even set.
  const activeFilterCount =
    (ticketFilter ? 1 : 0) +
    (tagFilter ? 1 : 0) +
    (epicFilter ? 1 : 0) +
    (labelFilter ? 1 : 0) +
    (showArchived ? 1 : 0) +
    (mineOnly ? 1 : 0)

  const clearFilters = useCallback(() => {
    setTicketFilter('')
    setTagFilter('')
    setEpicFilter('')
    setLabelFilter('')
    setShowArchived(false)
    setMineOnly(false)
  }, [])
  const signOut = useCallback(() => {
    saveToken('')
    setToken('')
  }, [])
  const [inboxOpen, setInboxOpen] = useState(false)
  const [me, setMe] = useState({ actor: '', scopes: [] as string[], expertise: [] as string[] })

  const [detail, setDetail] = useState<Ticket | null>(null)
  const [questions, setQuestions] = useState<Question[]>([])
  const [initiativesWaiting, setInitiativesWaiting] = useState(0)
  const [asking, setAsking] = useState(false)

  // The live indicator. `idle` before the first load, `live` once the event
  // cursor is moving, `reconnecting` when a poll failed — a board that has
  // quietly stopped updating looks exactly like a board with nothing happening,
  // which is the failure this makes visible.
  const [conn, setConn] = useState<'idle' | 'loading' | 'live' | 'reconnecting'>('idle')

  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { message?: string }
      if (isAuthError(e)) {
        saveToken('')
        setToken('')
        return
      }
      toast(err?.message || 'Request failed', 'err')
    },
    [toast],
  )

  useEffect(() => {
    if (!token) return
    let cancelled = false
    setProjectsStatus('loading')
    listProjects(token)
      .then((items) => { if (!cancelled) { setProjects(items); setProjectsStatus('ready') } })
      .catch((e) => { if (!cancelled) { setProjectsStatus('error'); handleErr(e) } })
    whoami(token)
      .then((w) => {
        const scopes = w.scopes ?? []
        setMe({
          actor: w.actor ?? '',
          scopes,
          // `mine` means "routed to my expertise" — the scopes carry it.
          expertise: scopes.filter((x) => x.startsWith('expert:')).map((x) => x.slice(7)),
        })
      })
      .catch(() => setMe({ actor: '', scopes: [], expertise: [] }))
    return () => { cancelled = true }
  }, [token, handleErr, projectRetry])

  const load = useCallback(async () => {
    if (!token || !effectiveProject) return
    setLoadError(false)
    setConn((c) => (c === 'live' ? c : 'loading'))
    const [wf, ts] = await Promise.all([
      getWorkflow(token, effectiveProject),
      listTickets(token, effectiveProject),
    ]).catch((error) => {
      if (activeScope.current === effectiveProject) setLoadError(true)
      throw error
    })
    if (activeScope.current !== effectiveProject) return
    setWorkflow(wf)
    setTickets(ts)
    setConn('live')
  }, [token, effectiveProject])

  useEffect(() => {
    if (!token || !effectiveProject) return
    setTickets([])
    setWorkflow(null)
    setRoadmapStatus('loading')
    setDetail(null)
    setSelectedId(deepTicket ?? null)
    setCreatingEpic(false)
    load().catch(handleErr)
  }, [token, effectiveProject, load, handleErr, deepTicket])

  // Who a question raised here can be addressed to. A failed read leaves the list
  // empty, which hides the control rather than offering names the server refuses.
  useEffect(() => {
    if (!token || !effectiveProject) {
      setAskPeople([])
      return
    }
    let cancelled = false
    listUsers(token, { project: effectiveProject, limit: 200 })
      .then((page) => {
        if (!cancelled) setAskPeople(page.items.map((u) => ({ handle: u.handle, label: u.label })))
      })
      .catch(() => {
        if (!cancelled) setAskPeople([])
      })
    return () => {
      cancelled = true
    }
  }, [token, effectiveProject])

  // Live updates by polling the event log. `EventSource` cannot set an
  // Authorization header, which is why this is a poll and not the SSE stream.
  useEffect(() => {
    if (!token || !effectiveProject) return
    const id = window.setInterval(() => {
      getEvents(token, cursor)
        .then((page) => {
          if (page.cursor != null) setCursor(page.cursor)
          setConn('live')
          if (hasEvents(page)) {
            void load().catch(handleErr)
            setEpoch((n) => n + 1)
          }
        })
        // A failed poll is not an error to shout about, but the board must stop
        // claiming to be live — silently stale is the failure worth surfacing.
        //
        // An AUTH failure is a different thing entirely and used to land here
        // too: a revoked or expired token read as "reconnecting" forever, so the
        // viewer sat looking at stale tickets that would never update, never
        // told to re-authenticate. A dead credential is not a flaky network.
        .catch((e) => {
          if (isAuthError(e)) {
            handleErr(e)
            return
          }
          setConn('reconnecting')
        })
    }, POLL_MS)
    return () => window.clearInterval(id)
  }, [token, effectiveProject, cursor, load, handleErr])

  // The roadmap, fetched only while the epics view is open: it runs a query per
  // epic, so a reader on the board should not pay for it. A failed read is
  // explicit and retryable; it must never look like a project without epics.
  useEffect(() => {
    if (!token || !effectiveProject || view !== 'epics') return
    let cancelled = false
    // Preserve the mounted view during background refreshes so filters survive.
    setRoadmapStatus((status) => status === 'error' ? (availableRoadmap.current ? 'ready' : 'loading') : status)
    fetchRoadmap(token, effectiveProject)
      .then((rm) => {
        if (!cancelled) { setRoadmapResult({ project: effectiveProject, data: rm }); setRoadmapStatus('ready') }
      })
      .catch((e) => {
        if (isAuthError(e)) {
          handleErr(e)
          return
        }
        if (!cancelled) { setRoadmapStatus('error') }
      })
    return () => {
      cancelled = true
    }
  }, [token, effectiveProject, view, epoch, handleErr])

  // The open drawer refreshes with the board.
  //
  // `detail` used to be fetched once, in `openTicket`, so polling updated the
  // card BEHIND the drawer — state, claim, blocked chip — while the drawer kept
  // rendering its open-time snapshot. The two then disagreed on screen at the
  // same time, which is worse than either being stale alone.
  useEffect(() => {
    if (!token || !selectedId || !detail) return
    let cancelled = false
    const request = detailRequest.current
    getTicket(token, selectedId)
      .then((ticket) => { if (!cancelled && detailRequest.current === request && detailContext.current.token === token && detailContext.current.project === effectiveProject) setDetail(ticket) })
      // A failed refresh leaves the drawer on what it had; the next tick retries.
      .catch(() => {})
    return () => { cancelled = true }
    // `tickets` is the signal that something changed — the poll replaces it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, effectiveProject, selectedId, tickets])

  // Open questions per ticket — what the detail drawer's callout counts.
  useEffect(() => {
    if (!token || !effectiveProject) return
    listQuestions(token, { project: effectiveProject, status: 'open' })
      .then(setQuestions)
      .catch(() => setQuestions([]))
  }, [token, effectiveProject, tickets])

  // How many initiatives want a person — the rail's badge, and the only thing
  // the board reads from that surface.
  //
  // Keyed on `epoch` rather than on a timer of its own: appending a note or
  // deciding an amendment emits an event, so the poll that already runs is the
  // signal, and a quiet board makes no requests for this at all. A failure
  // clears the badge instead of freezing the last number — a stale count would
  // send someone looking for work that is already done.
  useEffect(() => {
    if (!token || !effectiveProject) return
    listInitiatives(token, { project: effectiveProject, limit: 200 })
      .then((page) => setInitiativesWaiting(countWaiting(page.items ?? [])))
      .catch(() => setInitiativesWaiting(0))
  }, [token, effectiveProject, epoch])

  const questionsByTicket = useMemo(() => {
    const m = new Map<string, { count: number; blocking: number; advisory: number; conv: number }>()
    for (const q of questions) {
      const e = m.get(q.ticket) ?? { count: 0, blocking: 0, advisory: 0, conv: 0 }
      e.count++
      if (q.awaiting === 'agent') e.conv++
      else if (q.mode === 'advisory') e.advisory++
      else e.blocking++
      m.set(q.ticket, e)
    }
    return m
  }, [questions])

  const index = useMemo(() => indexById(tickets), [tickets])

  // Filters compose: a ticket must satisfy every active one. `inSubtree` is what
  // makes "filter by TK-7" keep TK-7's subtasks visible instead of orphaning them.
  const visible = useMemo(() => {
    // Archived tickets are hidden unless asked for — they are still real work
    // that happened, so they are excluded, not deleted.
    let out = showArchived ? tickets : tickets.filter((x) => !x.archived_at)
    if (ticketFilter) out = out.filter((x) => inSubtree(x, ticketFilter, index))
    if (tagKind) out = out.filter((x) => matchesTagRefs(x.tags, tagKind, tagFilter))
    if (epicFilter) out = out.filter((x) => epicOf(x, index) === epicFilter)
    if (labelFilter) out = out.filter((x) => (x.labels ?? []).includes(labelFilter))
    // "mine": claimed by me, or carrying a tag my expertise routes on.
    if (mineOnly && me.actor) {
      out = out.filter(
        (x) =>
          x.claim?.holder === me.actor ||
          (x.tags ?? []).some((tag) => me.expertise.includes(tag)),
      )
    }
    return out
  }, [tickets, ticketFilter, tagKind, tagFilter, epicFilter, labelFilter, showArchived, mineOnly, me, index])

  const states = useMemo(() => workflow?.states?.map((s) => s.id) ?? [], [workflow])
  // The phone's column, resolved: an explicit pick if the reader made one and it
  // still exists in this project's workflow, otherwise the first state.
  const phoneState = (mobileState && states.includes(mobileState) ? mobileState : states[0]) ?? ''
  const columns = useMemo(() => {
    const m = new Map<string, Ticket[]>()
    for (const s of states) m.set(s, [])
    for (const x of visible) {
      if (!m.has(x.state)) m.set(x.state, [])
      m.get(x.state)!.push(x)
    }
    return m
  }, [visible, states])

  const epicGroups = useMemo(() => {
    if (!groupByEpic) return null
    const m = new Map<string, Ticket[]>()
    for (const x of visible) {
      const e = epicOf(x, index)
      if (!m.has(e)) m.set(e, [])
      m.get(e)!.push(x)
    }
    return m
  }, [groupByEpic, visible, index])

  const allTags = useMemo(
    () => [...new Set(tickets.flatMap((x) => x.tags ?? []))].sort(),
    [tickets],
  )
  // A tag is `kind:handle`. The KIND is a plain picker — a project has a handful
  // of kinds and searching them would be ceremony — while the VALUES get the
  // typeahead, because there can be hundreds.
  const tagKinds = useMemo(
    () => [...new Set(allTags.map((tag) => tag.split(':')[0] ?? ''))].filter(Boolean).sort(),
    [allTags],
  )
  const tagValues = useMemo(
    () => (tagKind ? allTags.filter((tag) => tag.startsWith(tagKind + ':')) : allTags),
    [allTags, tagKind],
  )
  const allLabels = useMemo(
    () => [...new Set(tickets.flatMap((x) => x.labels ?? []))].sort(),
    [tickets],
  )
  const epics = useMemo(
    () => tickets.filter((x) => x.type === 'epic').map((x) => ({ id: x.id, title: x.title })),
    [tickets],
  )

  // The card is already loaded; the drawer wants deps and links too, so it
  // re-reads the one ticket rather than fattening the list request for all.
  const openTicket = useCallback(
    (id: string) => {
      const request = ++detailRequest.current
      setSelectedId(id)
      const known = tickets.find((x) => x.id === id) ?? null
      setDetail(known)
      if (token) getTicket(token, id).then((ticket) => { if (detailRequest.current === request && detailContext.current.token === token && detailContext.current.project === effectiveProject) setDetail(ticket) }).catch(() => {})
    },
    [tickets, token, effectiveProject],
  )

  const currentProject = projects.find((p) => p.id === effectiveProject) as
    | (Project & Record<string, unknown>)
    | undefined

  if (!token) {
    return (
      <TokenGate
        title={`takomo · ${surface}`}
        subtitle={surface === 'epics' ? t.epGateSub : t.gateSub}
        tokenLabel={t.gateTokenLabel}
        openLabel={surface === 'epics' ? t.epGateOpen : t.gateOpen}
        emptyMessage={t.typeFirst}
        initialToken={token}
        onSubmit={(tk) => {
          saveToken(tk)
          setToken(tk)
          // The project list arrives via the effect above; nothing is chosen on
          // the viewer's behalf here.
        }}
      />
    )
  }

  return (
    <AppShell
      lang={lang}
      onLang={(l) => { setLang(l); localStorage.setItem(LS_LANG, l) }}
      rail={{
        onNavigate: navigate,
        current: surface,
        nav: {
          board: t.board,
          epics: t.epics,
          inbox: t.inbox,
          specification: t.specification,
          initiatives: t.initiatives,
          schedules: t.schedules,
          environments: t.environments,
        },
        // The board already loads this project's open questions for its own
        // inbox drawer, so the rail can badge /inbox without a second request.
        // Initiatives are a second request, and worth one: a document waiting on
        // a decision is invisible from here otherwise, and the board is where a
        // day starts.
        badges: { inbox: questions.length, initiatives: initiativesWaiting },
        projects: projects.map(({ id, name, archived, archived_at }) => ({
          id,
          name,
          archived,
          archived_at,
        })),
        project: effectiveProject,
        onProject: (id) => {
          // An explicit pick DOES change the shared selection — that is a human
          // saying which project they mean, on every surface.
          setProject(id)
          saveProject(id)
          setTicketFilter('')
          setTagFilter('')
        },
        // No "all projects" entry here on purpose: a kanban's columns come from
        // ONE project's workflow, and two projects need not agree on their states.
        projectLabels: { project: t.project, search: t.projectSearch, noMatch: t.projectNoMatch },
        labels: {
          expand: t.navExpand,
          collapse: t.navCollapse,
          signOut: t.signout,
          account: t.navAccount,
          settings: t.settings,
        },
        collapsed: navCollapsed,
        onCollapsed: setNavCollapsed,
        actor: me.actor,
        scopes: me.scopes,
        onSignOut: signOut,
      }}
    >
      {view === 'board' && <AppHeader
        title={t.board}
      >
        {/* Why nothing on this board can be changed. The board itself only
            reads, so the freeze would otherwise be invisible here until someone
            tried to write from another surface and got a 409 with no context. */}
        {currentProject?.archived === true && (
          <Hint text={t.projArchivedHint}>
            <span
              className="border-border text-muted-foreground rounded-lg border px-2 py-1 text-[12px] font-[650]"
            >
              {t.projArchived}
            </span>
          </Hint>
        )}
        {/* The filter bank collapses on a phone.
            Measured: at 375px the header was 355px tall — 44% of the viewport —
            leaving 457px of board and 1.3 of 8 columns visible. Hiding these
            behind a toggle returns roughly a third of the screen, and they are
            the controls a phone user reaches for least. */}
        <button
          type="button"
          onClick={() => setFiltersOpen((v) => !v)}
          aria-expanded={filtersOpen}
          className="text-muted-foreground border-border cursor-pointer rounded-lg border px-3 py-2 text-[13px] font-[650] md:hidden"
        >
          {t.filters}
          {activeFilterCount > 0 && (
            <span className="bg-primary text-primary-foreground ml-1.5 inline-block min-w-[17px] rounded-[9px] px-1.25 text-center text-[11px] font-bold">
              {activeFilterCount}
            </span>
          )}
        </button>
        <div
          className={cn(
            'flex flex-wrap items-center gap-2.5',
            filtersOpen ? 'flex w-full md:w-auto' : 'hidden md:flex',
          )}
        >
        <Typeahead
          id="tickfilter"
          options={tickets.map((x) => ({ id: x.id, title: x.title }))}
          value={ticketFilter}
          onChange={setTicketFilter}
          labels={{
            all: t.allTickets,
            placeholder: t.taTicket,
            clear: t.taClear,
            noMatch: t.taNoMatch,
            count: t.taCount,
            count1: t.taCount1,
        countTruncated: t.taCountMore,
          }}
        />
        {/* The SAME control as the ticket filter, mounted again — see
            components/Typeahead.tsx. Two mount points, one implementation. */}
        <Picker
          id="tagkindsel"
          aria-label={t.tagsHdr}
          value={tagKind}
          onValueChange={(v) => {
            setTagKind(v)
            // The old value belongs to the old kind; keeping it would filter to
            // an empty board with no visible reason.
            setTagFilter('')
          }}
          className="bg-muted text-foreground border-border cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
          options={[
            { value: '', label: t.allTags },
            ...tagKinds.map((k) => ({ value: k, label: k })),
          ]}
        />
        <Typeahead
          id="tagvalfilter"
          options={tagValues.map((tag) => ({ id: tag }))}
          value={tagFilter}
          onChange={setTagFilter}
          labels={{
            all: t.allValues,
            placeholder: t.taTagValue,
            clear: t.taClear,
            noMatch: t.taNoMatch,
            count: t.taCount,
            count1: t.taCount1,
        countTruncated: t.taCountMore,
          }}
        />
        <Typeahead
          id="epicfilter"
          options={epics}
          value={epicFilter}
          onChange={setEpicFilter}
          labels={{
            all: t.allEpics,
            placeholder: t.allEpics,
            clear: t.taClear,
            noMatch: t.taNoMatch,
            count: t.taCount,
            count1: t.taCount1,
        countTruncated: t.taCountMore,
          }}
        />
        <Typeahead
          id="labelfilter"
          options={allLabels.map((l) => ({ id: l }))}
          value={labelFilter}
          onChange={setLabelFilter}
          labels={{
            all: t.allLabels,
            placeholder: t.taLabel,
            clear: t.taClear,
            noMatch: t.taNoMatch,
            count: t.taCount,
            count1: t.taCount1,
        countTruncated: t.taCountMore,
          }}
        />
        <label className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]">
          <Checkbox
            checked={groupByEpic}
            onCheckedChange={(e) => setGroupByEpic(e === true)}
          />
          {t.groupEpic}
        </label>
        <label className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]">
          <Checkbox
            checked={showArchived}
            onCheckedChange={(e) => setShowArchived(e === true)}
          />
          {t.archived}
        </label>
        {me.expertise.length > 0 && (
          <label className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]">
            <Checkbox
              checked={mineOnly}
              onCheckedChange={(e) => setMineOnly(e === true)}
            />
            {t.mine}
          </label>
        )}
        </div>
        <Button variant="outline" size="sm" onClick={() => setInboxOpen(true)}>
          {t.fullInbox}
          {questions.length > 0 && (
            <span className="bg-primary text-primary-foreground ml-1.5 rounded-[9px] px-1.5 text-[11px] font-bold">
              {questions.length}
            </span>
          )}
        </Button>
        {/* Live status. A board that has quietly stopped updating looks exactly
            like a board with nothing happening — this is what tells them apart. */}
        <Hint text={conn === 'live' ? t.live : conn === 'reconnecting' ? t.reconnecting : t.loading}>
          <span
            role="status"
            aria-label={conn === 'live' ? t.live : conn === 'reconnecting' ? t.reconnecting : t.loading}
            className={cn(
              'size-2 rounded-full',
              conn === 'live' && 'bg-ok',
              conn === 'reconnecting' && 'bg-crit',
              (conn === 'idle' || conn === 'loading') && 'bg-muted-foreground',
            )}
          />
        </Hint>
        {/* Project configuration lives in /settings now, not in a dialog here.
            A board is for looking at tickets; the page you go to in order to
            change how a project behaves is the settings page, and half the
            settings in each place was the split worth ending. */}
        <Hint text={t.settings}>
          <Button
            variant="outline"
            size="icon"
            onClick={() => navigate(`/settings?project=${encodeURIComponent(effectiveProject)}`)}
          >
            ⚙
          </Button>
        </Hint>
        <Hint text={t.refresh}>
          <Button variant="outline" size="icon"onClick={() => void load()}>
            ↻
          </Button>
        </Hint>
      </AppHeader>}

      {/* One state at a time on a phone. Rendered outside <main> so it does not
          scroll away with the columns. */}
      {view === 'board' && states.length > 0 && (
        <div className="border-b-border-soft flex gap-1 overflow-x-auto border-b px-3 py-2 md:hidden">
          {states.map((s) => {
            const n = (columns.get(s) ?? []).length
            return (
              <button
                key={s}
                type="button"
                onClick={() => setMobileState(s)}
                aria-current={s === phoneState}
                className={cn(
                  'shrink-0 cursor-pointer rounded-lg px-3 py-2 text-[12.5px] font-[650] uppercase tracking-[0.04em]',
                  s === phoneState ? 'bg-secondary text-primary' : 'text-muted-foreground',
                )}
              >
                {s}
                <span className="ml-1.5 tabular-nums">{n}</span>
              </button>
            )
          })}
        </div>
      )}

      <main className="min-h-0 flex-1 overflow-x-auto p-3">
        {/* Filtered to nothing: the board used to render its normal columns all
            reading 0, with no statement that a filter caused it and no way to
            undo them together. */}
        {view === 'epics' ? (
          projectsStatus === 'loading' ? <p role="status" className="text-muted-foreground p-6">{t.loading}</p> :
          projectsStatus === 'error' ? <div role="alert" className="p-6"><p>{t.epLoadError}</p><Button variant="outline" className="mt-3" onClick={() => setProjectRetry((n) => n + 1)}>{t.epRetry}</Button></div> :
          !effectiveProject ? <p className="text-muted-foreground p-6">{t.epNoProject}</p> :
          loadError && !workflow ? <div role="alert" className="p-6"><p>{t.epLoadError}</p><Button variant="outline" className="mt-3" onClick={() => { void load().catch(handleErr); setEpoch((n) => n + 1) }}>{t.epRetry}</Button></div> :
          (roadmapStatus === 'loading' || ((!roadmap || !workflow) && roadmapStatus !== 'error')) ? <p role="status" className="text-muted-foreground p-6">{t.loading}</p> :
          roadmapStatus === 'error' && !roadmap ? <div role="alert" className="p-6"><p>{t.epLoadError}</p><Button variant="outline" className="mt-3" onClick={() => setEpoch((n) => n + 1)}>{t.epRetry}</Button></div> :
          <>
          {(roadmapStatus === 'error' || loadError) && <div role="alert" className="mb-3 flex flex-wrap items-center gap-2 text-sm"><p>{t.epRefreshError}</p><Button variant="outline" size="sm" onClick={() => { void load().catch(handleErr); setEpoch((n) => n + 1) }}>{t.epRetry}</Button></div>}
          <EpicsView
            key={effectiveProject}
            onCreate={() => setCreatingEpic(true)}
            canCreate={(me.scopes.includes('write') || me.scopes.includes('admin')) && currentProject?.archived !== true}
            epics={roadmap?.epics ?? []}
            laneTitles={laneTitles(roadmap)}
            onOpen={openTicket}
            terminalStates={workflow?.states.filter((s) => s.terminal).map((s) => s.id)}
            labels={EPICS_STR[lang]}
          />
          </>
        ) : visible.length === 0 && tickets.length > 0 && activeFilterCount > 0 ? (
          <div className="text-muted-foreground px-2 py-14 text-center">
            <div className="text-[13.5px]">{t.noMatchFilters}</div>
            <button
              type="button"
              onClick={clearFilters}
              className="text-primary mt-2 cursor-pointer px-2 py-2 text-[13px] font-[650] underline"
            >
              {t.clearFilters} ({activeFilterCount})
            </button>
          </div>
        ) : epicGroups ? (
          <div className="flex flex-col gap-4">
            {[...epicGroups.entries()].map(([epic, ts]) => (
              <div key={epic || '(none)'}>
                <div className="text-muted-foreground mb-2 px-1 text-[11.5px] font-[750] tracking-[0.06em] uppercase">
                  {epic ? (index[epic]?.title ?? epic) : t.noEpic}
                </div>
                <div className="flex gap-3">
                  {states
                    .filter((s) => !isPhone || s === phoneState)
                    .map((s) => (
                    <Column
                      key={s}
                      state={s}
                      tickets={ts.filter((x) => x.state === s)}
                      selectedId={selectedId}
                      labels={{ showMore: t.showMore, blocked: t.blockedN, fromSchedule: t.fromSchedule, notFulfilled: t.notFulfilled }}
                      isDone={workflow?.states?.find((w) => w.id === s)?.terminal}
                      onOpen={openTicket}
              onNavigate={navigate}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="flex h-full min-h-0 gap-3">
            {[...columns.entries()]
              // On a phone only the selected state is mounted — not merely
              // hidden — so its cards are the only ones rendered.
              .filter(([state]) => !isPhone || state === phoneState)
              .map(([state, ts]) => (
              <Column
                key={state}
                state={state}
                tickets={ts}
                selectedId={selectedId}
                labels={{ showMore: t.showMore, blocked: t.blockedN, fromSchedule: t.fromSchedule, notFulfilled: t.notFulfilled }}
                isDone={workflow?.states?.find((w) => w.id === state)?.terminal}
                onOpen={openTicket}
              onNavigate={navigate}
              />
            ))}
          </div>
        )}
      </main>

      <CreateEpicDialog
        key={effectiveProject}
        open={creatingEpic}
        onOpenChange={setCreatingEpic}
        token={token}
        project={effectiveProject}
        lang={lang}
        onCreated={(ticket) => {
          if (activeScope.current !== effectiveProject) return
          setCreatingEpic(false)
          setTickets((items) => [...items, ticket])
          detailRequest.current++
          setSelectedId(ticket.id)
          setDetail(ticket)
          setEpoch((n) => n + 1)
        }}
      />

      <DetailPanel
        ticket={detail}
        questions={detail ? questionsByTicket.get(detail.id) : undefined}
        canAsk
        labels={{
          state: t.state,
          claimedBy: t.claimedBy,
          labels: t.labels,
          tagsHdr: t.tagsHdr,
          description: t.description,
          noDescription: t.noDescription,
          dependencies: t.dependencies,
          blockedByRel: t.blockedByRel,
          links: t.links,
          blockedN: t.blockedN,
          answeringResumes: t.answeringResumes,
          decisionRouted: t.decisionRouted,
          answerInInbox: t.answerInInbox,
          inConvN: t.inConvN,
          inConvSub: t.inConvSub,
          readThread: t.readThread,
          askHuman: t.askHuman,
          close: t.close,
          promotions: t.promotions,
          comments: t.comments,
          noComments: t.noComments,
          refLabel: t.refLabel,
          agoSep: t.agoSep,
        }}
        onClose={() => {
          detailRequest.current++
          setDetail(null)
          setSelectedId(null)
        }}
        onAsk={() => setAsking(true)}
      />

      <AskDrawer
        open={asking}
        onOpenChange={setAsking}
        ticket={detail?.id ?? ''}
        languageHint={(currentProject?.question_language as string | undefined) ?? undefined}
        people={askPeople}
        onAsk={async (fields) => {
          await askQuestion(token, fields)
          setAsking(false)
          toast(t.askHuman, 'success')
          const qs = await listQuestions(token, { project: effectiveProject, status: 'open' }).catch(() => [])
          setQuestions(qs)
        }}
        labels={{
          title: t.askHuman,
          subtitle: t.answeringResumes,
          fTicket: t.refLabel,
          fKind: t.question1,
          fMode: t.state,
          fTitle: t.description,
          fBody: t.notePlaceholder,
          fOptions: t.allValues,
          fOptionsHint: t.taAnyOf,
          fExpertise: t.tagsHdr,
          fExpertiseHint: t.taAddMore,
          fAssignee: t.askAssignee,
          fAssigneeHint: t.askAssigneeHint,
          fAssigneeAnyone: t.askAssigneeAnyone,
          blocking: t.blocking,
          advisory: t.advisory,
          blockingHint: t.answeringResumes,
          advisoryHint: t.decisionRouted,
          langHint: t.askLangHint,
          ask: t.send,
          cancel: t.cancel,
          needTitle: t.typeFirst,
        }}
      />

      <InboxDrawer
        open={inboxOpen}
        questions={questions}
        canAnswer={me.scopes.includes('human')}
        onClose={() => setInboxOpen(false)}
        onAnswer={async (q, value, note) => {
          await answerQuestion(token, q.id, { value, note: note || undefined })
          const qs = await listQuestions(token, { project: effectiveProject, status: 'open' }).catch(() => [])
          setQuestions(qs)
          void load()
        }}
        labels={{
          title: t.fullInbox,
          empty: t.allClear,
          emptySub: t.noneForTicket,
          blocking: t.blocking,
          advisory: t.advisory,
          inConversation: t.inConversation,
          awaiting: t.awaiting,
          awaitingSub: t.awaitingSub,
          recommends: t.recommends,
          notePlaceholder: t.notePlaceholder,
          send: t.send,
          cantAnswer: t.cantAnswer,
          close: t.close,
          approve: t.approve,
          reject: t.reject,
          yes: t.yes,
          no: t.no,
          writeOwn: t.customDivider,
          ownPlaceholder: t.customPlaceholder,
          textPlaceholder: t.answerPlaceholder,
          typeFirst: t.typeFirst,
          sendFirst: t.sendFirst,
        }}
      />

    </AppShell>
  )
}
