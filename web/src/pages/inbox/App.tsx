// /inbox — where an agent's question reaches a person.
//
// A blocking question parks its ticket and releases the agent's lease; answering
// resumes it, but only once every open blocking question on that ticket is
// answered. An advisory question records a routed decision and changes no state.
//
// Answering is one press followed by a 30-second undo window (lib/undo-queue.ts).
// The item leaves Open immediately and the write happens when the window closes,
// so working through a full inbox never waits on the network.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useNavigate } from 'react-router'
import { cn } from '@/lib/utils'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { AnswerLinkDialog } from '@/components/inbox/AnswerLinkDialog'
import { FilterBar } from '@/components/inbox/FilterBar'
import { FolderRail } from '@/components/inbox/FolderRail'
import { QuestionRow } from '@/components/inbox/QuestionRow'
import { ReadingPane } from '@/components/inbox/ReadingPane'
import { UndoSnackbar } from '@/components/inbox/UndoSnackbar'
import { EpicGroupHeader } from '@/components/inbox/EpicGroupHeader'
import { useUndoQueue } from '@/hooks/useUndoQueue'
import {
  activeFilterCount,
  filterQuestions,
  groupByEpic,
  sortForFolder,
  type EpicGroup,
} from '@/lib/question-filters'
import { clearedFilters, readView, writeView, type InboxView } from '@/lib/inbox-url'
import { indexById } from '@/lib/tickets'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { listUsers } from '@/lib/users'
import { answerPayloadFor, displayValue, type Draft } from '@/lib/answers'
import { undoInto } from '@/lib/undo-queue'
import {
  FOLDERS,
  answerQuestion,
  assignQuestion,
  getThread,
  listQuestions,
  listTicketRefs,
  mintAnswerLink,
  reopenQuestion,
  sendFollowup,
  withdrawQuestion,
  type AnswerLink,
  type Folder,
  type Question,
  type ThreadMessage,
  type TicketRef,
} from '@/lib/questions'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const LS_COLLAPSED = 'takomo.inbox.collapsed'
const POLL_MS = 5000

/** The folded-away epics, from the last visit. A corrupt value is not fatal. */
function loadCollapsed(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_COLLAPSED) ?? '[]')
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => loadProject())
  const [gateError, setGateError] = useState('')

  const [me, setMe] = useState({ actor: '', scopes: [] as string[], handle: '' })
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [projects, setProjects] = useState<Project[]>([])
  const [questions, setQuestions] = useState<Question[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [thread, setThread] = useState<ThreadMessage[]>([])
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const [link, setLink] = useState<AnswerLink | null>(null)
  const [tickets, setTickets] = useState<TicketRef[]>([])
  const [filtersOpen, setFiltersOpen] = useState(false)

  // The whole view — folder, every filter, and the grouping — read from the URL
  // and written back to it, so it survives a reload and can be sent to someone.
  // Initialised from `window.location` rather than from a default: a link
  // arriving with filters must open showing them, not flash the unfiltered
  // inbox first.
  const [view, setView] = useState<InboxView>(() => readView(window.location.search))
  const { folder, group } = view
  const updateView = useCallback((p: Partial<InboxView>) => {
    setView((v) => ({ ...v, ...p }))
    // Any change to what the list contains invalidates the selection: the
    // question you were reading may not be in the new view at all.
    setSelectedId(null)
  }, [])
  // Clearing REPLACES the view rather than merging into it. `clearedFilters`
  // expresses "gone" by omitting a key, and a partial merge reads an absent key
  // as "unchanged" — so routing it through `updateView` cleared nothing at all.
  const resetFilters = useCallback(() => {
    setView(clearedFilters)
    setSelectedId(null)
  }, [])

  // Which epics are folded away. Persisted, because a reader who collapsed the
  // epic they are not working on this week means it for longer than one tab.
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadCollapsed())

  const t = useMemo(() => pick(STR, lang), [lang])
  const canAnswer = me.scopes.includes('human')
  // The tags an `expert:<tag>` scope routes questions on — what "mine" means.
  const expertise = useMemo(
    () => me.scopes.filter((s) => s.startsWith('expert:')).map((s) => s.slice(7)),
    [me.scopes],
  )

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

  // The queue re-applies pending answers to every freshly loaded list; without
  // that an item pops back into Open while its own snackbar is counting down.
  const queue = useUndoQueue({
    commit: (p) => answerQuestion(token, p.qid, p.payload),
    refresh: () => void fetchAll(),
    onError: handleErr,
  })
  const applyRef = useRef(queue.apply)
  applyRef.current = queue.apply

  const fetchAll = useCallback(async () => {
    const list = await listQuestions(token, { project })
    setQuestions(applyRef.current(list, me.actor))
  }, [token, project, me.actor])

  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const [who, ps] = await Promise.all([
          whoami(token),
          listProjects(token).catch(() => [] as Project[]),
        ])
        if (cancelled) return
        setMe({
          actor: who.actor ?? '',
          scopes: who.scopes ?? [],
          // The person behind the credential. '' for a machine token, which is
          // what makes "for me" fall back to expertise alone.
          handle: who.user?.handle ?? '',
        })
        setProjects(ps)
      } catch (e) {
        if (!cancelled) handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, handleErr])

  useEffect(() => {
    if (!token) return
    fetchAll().catch(handleErr)
  }, [token, fetchAll, handleErr])

  // Tickets are per-project, so a filter carried across a project switch would
  // only ever show an empty inbox — the list is refetched and the filter cleared.
  useEffect(() => {
    if (!token) return
    let cancelled = false
    listTicketRefs(token, project)
      .then((ts) => !cancelled && setTickets(ts))
      .catch(() => !cancelled && setTickets([]))
    return () => {
      cancelled = true
    }
  }, [token, project])

  // Poll, but never while an undo window is open: a refetch mid-countdown is
  // exactly the case the queue's re-apply exists for, and not fighting it is
  // cheaper than relying on it.
  useEffect(() => {
    if (!token || queue.pending.length) return
    // A transient poll failure is genuinely not worth a toast — the next tick
    // fixes it. An AUTH failure is, and swallowing it meant a revoked token left
    // the inbox looking perfectly normal while showing questions that would
    // never refresh, and accepting answers that would fail on submit.
    const id = window.setInterval(
      () =>
        void fetchAll().catch((e) => {
          if (isAuthError(e)) handleErr(e)
        }),
      POLL_MS,
    )
    return () => window.clearInterval(id)
  }, [token, queue.pending.length, fetchAll, handleErr])

  // The ticket tree, for the subtree filter and the epic grouping. Rebuilt only
  // when the ticket list is refetched, not on every keystroke in the filter bar.
  const index = useMemo(() => indexById(tickets), [tickets])

  // `visible` — everything the current filters admit. The folder split and the
  // counts both read from it, so a filtered-out question cannot be counted in a
  // folder it is not listed in.
  const visible = useMemo(
    () =>
      filterQuestions(questions, view, {
        index,
        expertise,
        handle: me.handle || undefined,
      }),
    [questions, view, index, expertise, me.handle],
  )
  const inFolder = useMemo(
    () => sortForFolder(visible.filter((q) => q.status === folder), folder),
    [visible, folder],
  )
  // Grouped, or one unnamed group holding everything — so the list below has
  // ONE shape to render and the keyboard has one order to walk.
  const groups: EpicGroup[] = useMemo(
    () =>
      group
        ? groupByEpic(inFolder, index)
        : [{ epic: '', title: '', questions: inFolder }],
    [group, inFolder, index],
  )
  // What j/k walks and what "select the next one after answering" means: the
  // rows a reader can actually see. A collapsed group is not skipped over
  // silently — it is not in the list at all.
  const walkable = useMemo(
    () =>
      group
        ? groups.filter((g) => !collapsed.has(g.epic)).flatMap((g) => g.questions)
        : inFolder,
    [group, groups, collapsed, inFolder],
  )
  // "Collapse all" flips to "Expand all" once nothing is left to fold — the
  // control names what it will DO, not what it did.
  const allCollapsed = groups.length > 0 && groups.every((g) => collapsed.has(g.epic))
  const askers = useMemo(
    () => [...new Set(questions.map((q) => q.asked_by ?? '').filter(Boolean))].sort(),
    [questions],
  )
  /**
   * Whether this reader can be routed to at all — by name, or by expertise. What
   * decides whether the "for me" toggle is offered: to a machine token with no
   * expert scope it could only ever empty the list.
   */
  const routable = expertise.length > 0 || me.handle !== ''
  /**
   * The people some question in view is waiting on, for the assignee picker.
   *
   * Derived from the questions already loaded rather than from `GET /v1/users`,
   * deliberately: the useful filter is "whose queue, among the ones with work
   * here", and a directory of fifty people where three have open questions makes
   * the reader hunt. Directory-wide assignment is the reading pane's job, and it
   * fetches for that.
   */
  const assignees = useMemo(() => {
    const seen = new Map<string, string>()
    for (const q of questions) {
      const who = q.assignee
      if (who?.handle && !seen.has(who.handle)) seen.set(who.handle, who.label)
    }
    return [...seen].map(([handle, label]) => ({ handle, label })).sort((a, b) =>
      a.label.localeCompare(b.label),
    )
  }, [questions])
  const active = activeFilterCount(view)
  // The nav badge counts what is OPEN, unfiltered. The folder counts follow the
  // filters — they describe this view — but the badge is a claim about the
  // fleet, and a text search that made it read 0 would say the queue is empty
  // when it is only hidden.
  const openTotal = useMemo(() => questions.filter((q) => q.status === 'open').length, [questions])
  const counts = useMemo(() => {
    const c: Partial<Record<Folder, number>> = {}
    for (const f of FOLDERS) c[f] = visible.filter((q) => q.status === f).length
    return c
  }, [visible])

  // An explicit selection wins even when its group is collapsed — you asked for
  // that question. The FALLBACK only ever lands on a row you can see.
  const selected = useMemo(
    () => inFolder.find((q) => q.id === selectedId) ?? walkable[0] ?? null,
    [inFolder, walkable, selectedId],
  )

  /**
   * Who the selected question can be addressed to: the members of ITS project,
   * not of the page's project filter — the filter may be empty (every project)
   * while a question always belongs to exactly one, and the server refuses a
   * non-member.
   *
   * Fetched per project rather than once for the whole directory, because
   * membership is the thing being offered. A failure leaves the list empty, which
   * hides the control: better than offering names the server will refuse.
   */
  const [members, setMembers] = useState<{ handle: string; label: string }[]>([])
  const memberProject = selected?.project ?? ''
  useEffect(() => {
    if (!token || !memberProject) {
      setMembers([])
      return
    }
    let cancelled = false
    listUsers(token, { project: memberProject, limit: 200 })
      .then((page) => {
        if (cancelled) return
        setMembers(page.items.map((u) => ({ handle: u.handle, label: u.label })))
      })
      .catch(() => {
        if (!cancelled) setMembers([])
      })
    return () => {
      cancelled = true
    }
  }, [token, memberProject])

  // Deep-linkable: #q=<id> opens that question, and selecting one writes it back
  // so the URL is shareable and bookmarkable.
  useEffect(() => {
    const m = /(?:^#|&)q=([^&]+)/.exec(window.location.hash || '')
    if (m?.[1]) setSelectedId(decodeURIComponent(m[1]))
  }, [])
  // `replace`, and through the router.
  //
  // Assigning `window.location.hash` pushed a history entry React Router did
  // not create, leaving its own index bookkeeping stale — and it pushed ONE PER
  // SELECTION, so walking twenty questions with j/k meant twenty Back presses
  // before leaving the inbox. With one app the expectation is that Back returns
  // to the previous SURFACE, not the previously-read question.
  useEffect(() => {
    // Clearing the selection clears the hash too. Without that, closing a
    // question leaves `#q=…` in the URL and the next visit to /inbox reads it
    // back on mount — so a phone user, for whom the pane REPLACES the list,
    // lands straight in the last question they read instead of their inbox.
    // The pathname is given explicitly: `navigate({ hash: '' })` alone does not
    // reliably drop an existing fragment.
    //
    // The filters ride along in the SEARCH half, written by the same effect:
    // two effects writing the same URL raced, and whichever ran second dropped
    // the other's half.
    navigate(
      {
        pathname: '/inbox',
        search: writeView(view),
        hash: selectedId ? 'q=' + selectedId : '',
      },
      { replace: true },
    )
  }, [selectedId, view, navigate])

  useEffect(() => {
    if (!selected || !token) {
      setThread([])
      return
    }
    let cancelled = false
    getThread(token, selected.id)
      .then((th) => !cancelled && setThread(th))
      .catch(() => !cancelled && setThread([]))
    return () => {
      cancelled = true
    }
  }, [selected, token])

  // j / k move, ↵ answers — the whole point of an inbox is not reaching for the
  // mouse. Ignored while typing, or ↵ would submit a decision mid-sentence.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement | null)?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement)?.isContentEditable) return
      if (e.key === 'j' || e.key === 'k') {
        // Walks what is on screen: a collapsed epic's questions are not stepped
        // through invisibly.
        const i = walkable.findIndex((q) => q.id === selected?.id)
        const next = e.key === 'j' ? i + 1 : i - 1
        if (next >= 0 && next < walkable.length) setSelectedId(walkable[next]!.id)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [walkable, selected])

  const toggleEpic = useCallback((epic: string) => {
    setCollapsed((cur) => {
      const next = new Set(cur)
      if (!next.delete(epic)) next.add(epic)
      localStorage.setItem(LS_COLLAPSED, JSON.stringify([...next]))
      return next
    })
  }, [])

  const setAllCollapsed = useCallback(
    (all: boolean) => {
      // Only the epics on screen — collapsing "all" must not silently fold away
      // groups this filter happens to be hiding, which the reader would meet as
      // a collapsed inbox next time with no idea why.
      const next = all ? new Set(groups.map((g) => g.epic)) : new Set<string>()
      localStorage.setItem(LS_COLLAPSED, JSON.stringify([...next]))
      setCollapsed(next)
    },
    [groups],
  )

  const setDraft = (id: string, patch: Draft) =>
    setDrafts((d) => ({ ...d, [id]: { ...d[id], ...patch } }))

  function submit(q: Question) {
    const payload = answerPayloadFor(q, drafts[q.id])
    const decision = `${t.decision}: ${displayValue(payload.value, {
      yes: q.kind === 'approve' ? t.approve : t.confirm,
      no: q.kind === 'approve' ? t.holdApprove : t.holdConfirm,
    })}`
    const detail = q.mode === 'advisory' ? t.recorded : q.ticket + t.resumedInto
    queue.enqueue(q, payload, decision, detail)
    // It has left Open — move to the next one so the reader keeps going.
    const rest = walkable.filter((x) => x.id !== q.id)
    setSelectedId(rest[0]?.id ?? null)
    setQuestions((cur) => applyRef.current(cur, me.actor))
  }

  function signOut() {
    // Write pending answers while the token is still valid — after it is gone
    // they are a 401, not a decision.
    void queue.flushAll().finally(() => {
      saveToken('')
      setToken('')
      setQuestions([])
    })
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · inbox"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.typeFirst}
        error={gateError}
        onSubmit={(tk) => {
          saveToken(tk)
          setGateError('')
          setToken(tk)
        }}
      />
    )
  }

  const paneLabels = selected && {
    back: t.back,
    yes: selected.kind === 'approve' ? t.approve : t.confirm,
    no: selected.kind === 'approve' ? t.holdApprove : t.holdConfirm,
    writeOwn: t.customDivider,
    ownPlaceholder: t.customPlaceholder,
    textPlaceholder: t.typeAnswer,
    recommends: t.agentSuggests,
    submit: t.submit,
    sendFollow: t.sendFollow,
    askFollow: t.askFollow,
    followFirst: t.followFirst,
    to: t.to,
    typeFirst: t.typeFirst,
    sendFirst: t.sendFirst,
    share: t.share,
    withdraw: t.withdraw,
    reopen: t.reopen,
    closed: t.closed,
    advisory: t.advTag,
    askedBy: t.askedBy,
    readonly: t.readonly,
    waitingAgentPrefix: t.waitingAgent,
    waitingAgentSuffix: t.waitingAgent2,
    noReply: t.noReply,
    assignTo: t.assignTo,
    assignNobody: t.assignNobody,
    assignHint: t.assignHint,
  }

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'inbox',
        nav: {
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          schedules: t.schedules,
          verification: t.verification,
          environments: t.environments,
        },
        badges: { inbox: openTotal },
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
          // Tickets, epics and askers are all per-project, so every filter that
          // names one would only ever produce an empty inbox after the switch.
          resetFilters()
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
        actor: me.actor,
        scopes: me.scopes,
        onSignOut: signOut,
      }}
    >
      <AppHeader
        title={t.inbox}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        <span className="text-muted-foreground mr-1 hidden text-[11.5px] md:inline">{t.kbd}</span>
        <Button variant="outline" size="icon" title="Refresh" onClick={() => void fetchAll()}>
          ↻
        </Button>
      </AppHeader>

      {/* The filters, in their own row rather than in the shared header — see
          components/inbox/FilterBar.tsx. On a phone the reading pane REPLACES
          the list, so the bar goes with the list it filters. */}
      <FilterBar
        className={selectedId ? 'hidden md:flex' : 'flex'}
        tickets={tickets}
        ticket={view.ticket ?? ''}
        onTicket={(id) => updateView({ ticket: id || undefined })}
        search={view.search ?? ''}
        onSearch={(text) => updateView({ search: text || undefined })}
        urgency={view.urgency ?? []}
        onUrgency={(levels) => updateView({ urgency: levels.length ? levels : undefined })}
        mode={view.mode ?? ''}
        onMode={(m) => updateView({ mode: m || undefined })}
        mine={routable ? (view.mine ?? false) : undefined}
        onMine={routable ? (on) => updateView({ mine: on || undefined }) : undefined}
        hideAwaitingAgent={view.hideAwaitingAgent ?? false}
        onHideAwaitingAgent={(on) => updateView({ hideAwaitingAgent: on || undefined })}
        expiringSoon={view.expiringSoon ?? false}
        onExpiringSoon={(on) => updateView({ expiringSoon: on || undefined })}
        askers={askers}
        askedBy={view.askedBy ?? ''}
        onAskedBy={(a) => updateView({ askedBy: a || undefined })}
        assignees={assignees}
        assignee={view.assignee ?? ''}
        onAssignee={(h) => updateView({ assignee: h || undefined })}
        group={group}
        onGroup={(on) => updateView({ group: on })}
        matched={visible.length}
        activeCount={active}
        onClear={() => resetFilters()}
        open={filtersOpen}
        onOpen={setFiltersOpen}
        labels={{
          filters: t.filters,
          allTickets: t.allTickets,
          taTicket: t.taTicket,
          taClear: t.taClear,
          taNoMatch: t.taNoMatch,
          taCount: t.taCount,
          taCount1: t.taCount1,
          taCountMore: t.taCountMore,
          search: t.search,
          searchPlaceholder: t.searchPh,
          urgency: t.urgencyHdr,
          critical: t.crit,
          high: t.high,
          normal: t.normal,
          low: t.low,
          allModes: t.allModes,
          blocking: t.blockingF,
          advisory: t.advisoryF,
          mine: t.mine,
          mineHint: t.mineHint,
          waiting: t.waitingF,
          waitingHint: t.waitingHint,
          soon: t.soonF,
          soonHint: t.soonHint,
          allAskers: t.allAskers,
          asker: t.askerHdr,
          anyAssignee: t.anyAssignee,
          assignee: t.assigneeHdr,
          unassigned: t.unassigned,
          groupEpic: t.groupEpic,
          count: t.taQCount,
          count1: t.taQCount1,
          clearAll: t.clearAll,
        }}
      />

      {/* Stacked master/detail on a phone, three panes from `md` up.
          The fixed columns totalled 500px, so below ~540px the `1fr` reading
          pane resolved to literally 0px — and with the root `overflow-hidden`
          it could not even be scrolled to. Tapping a question did nothing
          visible. Measured 0px at every width from 320 to 430. */}
      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[180px_320px_1fr]">
        <FolderRail
          className="hidden md:block"
          folders={FOLDERS}
          current={folder}
          counts={counts}
          labels={{
            heading: t.folders,
            open: t.open,
            answered: t.answered,
            withdrawn: t.withdrawn,
            expired: t.expired,
          }}
          onSelect={(f) => updateView({ folder: f })}
        />

        <section
          className={cn(
            'bg-card border-r-border-soft min-h-0 overflow-y-auto border-r md:block',
            // On a phone the list and the reading pane occupy the same cell;
            // selecting a question swaps to it, and the pane's back button
            // swaps back. `#q=` is already in the URL, so Back works too.
            selectedId ? 'hidden' : 'block',
          )}
        >
          {/* The folder rail is hidden on a phone, so the folders come back as a
              chip row above the list — folders are how this surface is
              navigated, and losing them to a breakpoint would be worse than the
              cramped rail. */}
          <div className="border-b-border-soft flex gap-1 overflow-x-auto border-b px-3 py-2 md:hidden">
            {FOLDERS.map((f) => (
              <button
                key={f}
                type="button"
                onClick={() => updateView({ folder: f })}
                className={cn(
                  'shrink-0 cursor-pointer rounded-lg px-3 py-2 text-[13px] font-[650]',
                  f === folder ? 'bg-secondary text-primary' : 'text-muted-foreground',
                )}
              >
                {t[f] ?? f}
                {(counts[f] ?? 0) > 0 && (
                  <span className="ml-1.5 tabular-nums">{counts[f]}</span>
                )}
              </button>
            ))}
          </div>

          {inFolder.length === 0 ? (
            // "All clear — the fleet is working" is a claim about the SYSTEM,
            // and it was being made whenever the reader's own ticket filter
            // emptied the list, or whenever they opened a folder that simply
            // has nothing in it. Both are statements about the view, not the
            // fleet. `noneForTicket` was written for exactly this and had never
            // been wired up.
            <div className="text-muted-foreground px-6 py-14 text-center">
              {active > 0 ? (
                <>
                  <div className="text-[13px]">
                    {view.ticket ? t.noneForTicket : t.noneForFilters}
                  </div>
                  <button
                    type="button"
                    onClick={() => resetFilters()}
                    className="text-primary mt-2 cursor-pointer px-2 py-1 text-[13px] font-[650] underline"
                  >
                    {t.clearAll} ({active})
                  </button>
                </>
              ) : folder === 'open' ? (
                <>
                  <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.allClear}</div>
                  <div className="text-[13px]">{t.allClearSub}</div>
                </>
              ) : (
                <div className="text-[13px]">{t.folderEmpty}</div>
              )}
            </div>
          ) : group ? (
            <>
              {/* One control for the whole list, so a reader who wants an
                  overview does not click eleven headings to get one. */}
              <div className="border-b-border-soft flex items-center justify-end gap-3 border-b px-3.5 py-1.5">
                <button
                  type="button"
                  onClick={() => setAllCollapsed(!allCollapsed)}
                  className="text-muted-foreground hover:text-primary cursor-pointer text-[11.5px] font-[650]"
                >
                  {allCollapsed ? t.expandAll : t.collapseAll}
                </button>
              </div>
              {groups.map((g) => (
                <div key={g.epic || '(none)'}>
                  <EpicGroupHeader
                    title={g.epic ? g.title : t.noEpic}
                    count={g.questions.length}
                    collapsed={collapsed.has(g.epic)}
                    onToggle={() => toggleEpic(g.epic)}
                  />
                  {!collapsed.has(g.epic) &&
                    g.questions.map((q) => (
                      <QuestionRow
                        key={q.id}
                        question={q}
                        selected={q.id === selected?.id}
                        landed={queue.pending.some((p) => p.qid === q.id)}
                        labels={{ advisory: t.advTag, askedBy: t.askedBy, waitingAgent: t.stallTag, forPerson: t.forPerson }}
                        onSelect={setSelectedId}
                      />
                    ))}
                </div>
              ))}
            </>
          ) : (
            inFolder.map((q) => (
              <QuestionRow
                key={q.id}
                question={q}
                selected={q.id === selected?.id}
                landed={queue.pending.some((p) => p.qid === q.id)}
                labels={{ advisory: t.advTag, askedBy: t.askedBy, waitingAgent: t.stallTag, forPerson: t.forPerson }}
                onSelect={setSelectedId}
              />
            ))
          )}
        </section>

        {selected && paneLabels ? (
          <ReadingPane
            key={selected.id}
            className={selectedId ? 'flex' : 'hidden md:flex'}
            onBack={() => setSelectedId(null)}
            question={selected}
            thread={thread}
            draft={drafts[selected.id]}
            onDraft={(patch) => setDraft(selected.id, patch)}
            canAnswer={canAnswer}
            labels={paneLabels}
            onSubmit={() => submit(selected)}
            onFollowup={(text) =>
              sendFollowup(token, selected.id, text)
                .then(() => {
                  toast(t.followupSent, 'success')
                  return fetchAll()
                })
                .catch(handleErr)
            }
            onWithdraw={() =>
              withdrawQuestion(token, selected.id)
                .then(() => {
                  toast(t.withdrawn2, 'success')
                  return fetchAll()
                })
                .catch(handleErr)
            }
            onReopen={() =>
              reopenQuestion(token, selected.id)
                .then(() => {
                  toast(t.reopened, 'success')
                  return fetchAll()
                })
                .catch(handleErr)
            }
            onShare={() =>
              mintAnswerLink(token, selected.id)
                .then((l) => {
                  setLink(l)
                  toast(t.answerLinkCreated, 'success')
                })
                .catch(handleErr)
            }
            answerPending={queue.pending.some((p) => p.qid === selected.id)}
            assignable={canAnswer ? members : []}
            onAssign={(handle) =>
              assignQuestion(token, selected.id, handle)
                .then((r) => {
                  const who = r.question.assignee?.label
                  toast(
                    who ? t.assigned.replace('{who}', who) : t.unassignedDone,
                    'success',
                  )
                  return fetchAll()
                })
                .catch(handleErr)
            }
          />
        ) : (
          <div className="text-muted-foreground px-6 py-14 text-center">
            <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.nothingSel}</div>
            <div className="text-[13px]">{t.nothingSelSub}</div>
          </div>
        )}
      </main>

      <UndoSnackbar
        pending={queue.pending}
        now={queue.now}
        labels={{ undo: t.undo, seconds: 's' }}
        onUndo={(qid) => {
          // Take the snapshot back from the queue in the same breath as
          // cancelling — see `undo` for why it is returned rather than looked up.
          const cancelled = queue.undo(qid)
          if (cancelled) setQuestions((cur) => undoInto(cur, cancelled))
          toast(t.cancelled)
        }}
      />

      <AnswerLinkDialog
        link={link}
        lang={lang}
        onClose={() => setLink(null)}
        labels={{
          title: t.linkTitle,
          body: t.linkBody,
          once: t.linkOnce,
          copy: t.copy,
          copied: t.copied,
          done: t.done,
          validUntil: t.validUntil,
          copyFail: t.copyFail,
        }}
      />
    </AppShell>
  )
}
