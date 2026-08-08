// /board — one route, three audiences.
//
//   #a=tka_…  an outside expert answering ONE question (AnswerGrantPage)
//   #s=tks_…  a read-only share of a project or subtree (SharePage)
//   neither   the board itself, on a `tk_` token from localStorage
//
// The fragment wins over a stored token — see lib/board-mode.ts for why.
import { useCallback, useEffect, useMemo, useState } from 'react'

import { AppHeader } from '@/components/AppHeader'
import { useNavigate } from 'react-router'
import { loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { Typeahead } from '@/components/Typeahead'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Column } from '@/components/board/Column'
import { AskDrawer } from '@/components/board/AskDrawer'
import { DetailPanel } from '@/components/board/DetailPanel'
import { InboxDrawer } from '@/components/board/InboxDrawer'
import { SettingsSheet } from '@/components/board/SettingsSheet'
import { AnswerGrantPage } from './AnswerGrantPage'
import { SharePage } from './SharePage'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { modeFor } from '@/lib/board-mode'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { epicOf, inSubtree, indexById } from '@/lib/tickets'
import { cn } from '@/lib/utils'
import {
  getEvents,
  getTicket,
  getWorkflow,
  listTickets,
  type Ticket,
  type Workflow,
} from '@/lib/board'
import { answerQuestion, askQuestion, listQuestions, type Question } from '@/lib/questions'
import {
  saveProjectSettings,
  settingsFrom,
  type ProjectSettings,
} from '@/lib/project-settings'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const POLL_MS = 4000

export function App() {
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const t = useMemo(() => pick(STR, lang), [lang])

  // Read once: a grant is what the URL asked for at load, and re-reading it on
  // every render would fight the board's own hash writes.
  const [mode] = useState(() => modeFor(window.location.hash))

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

  return <Board lang={lang} setLang={setLang} deepTicket={mode.ticket} />
}

function Board({
  lang,
  setLang,
  deepTicket,
}: {
  lang: Locale
  setLang: (l: Locale) => void
  deepTicket?: string
}) {
  const navigate = useNavigate()
  const { toast } = useToast()
  const t = useMemo(() => pick(STR, lang), [lang])

  const [token, setToken] = useState(() => loadToken())
  const [project, setProject] = useState(() => loadProject())
  const [projects, setProjects] = useState<Project[]>([])

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
  const [workflow, setWorkflow] = useState<Workflow | null>(null)
  const [tickets, setTickets] = useState<Ticket[]>([])
  const [cursor, setCursor] = useState<number | string>(0)
  const [selectedId, setSelectedId] = useState<string | null>(deepTicket ?? null)

  const [ticketFilter, setTicketFilter] = useState(deepTicket ?? '')
  const [tagKind, setTagKind] = useState('')
  const [tagFilter, setTagFilter] = useState('')
  const [epicFilter, setEpicFilter] = useState('')
  const [labelFilter, setLabelFilter] = useState('')
  const [groupByEpic, setGroupByEpic] = useState(false)
  const [showArchived, setShowArchived] = useState(false)
  const [mineOnly, setMineOnly] = useState(false)
  const [inboxOpen, setInboxOpen] = useState(false)
  const [me, setMe] = useState({ actor: '', scopes: [] as string[], expertise: [] as string[] })

  const [detail, setDetail] = useState<Ticket | null>(null)
  const [questions, setQuestions] = useState<Question[]>([])
  const [asking, setAsking] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [settings, setSettings] = useState<ProjectSettings>(() => settingsFrom(undefined))
  const [origSettings, setOrigSettings] = useState<ProjectSettings>(() => settingsFrom(undefined))
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [saveErr, setSaveErr] = useState('')

  // The live indicator. `idle` before the first load, `live` once the event
  // cursor is moving, `reconnecting` when a poll failed — a board that has
  // quietly stopped updating looks exactly like a board with nothing happening,
  // which is the failure this makes visible.
  const [conn, setConn] = useState<'idle' | 'loading' | 'live' | 'reconnecting'>('idle')

  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { auth?: boolean; status?: number; message?: string }
      if (err?.auth || err?.status === 401 || err?.status === 403) {
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
    listProjects(token)
      .then(setProjects)
      .catch(() => setProjects([]))
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
  }, [token])

  const load = useCallback(async () => {
    if (!token || !effectiveProject) return
    setConn((c) => (c === 'live' ? c : 'loading'))
    const [wf, ts] = await Promise.all([
      getWorkflow(token, effectiveProject),
      listTickets(token, effectiveProject),
    ])
    setWorkflow(wf)
    setTickets(ts)
    setConn('live')
  }, [token, effectiveProject])

  // The project list is what makes `effectiveProject` resolvable, so it is
  // fetched whenever there is a token — not only from the token gate, which is
  // where it used to happen and which a returning viewer never sees.
  useEffect(() => {
    if (!token || projects.length) return
    listProjects(token).then(setProjects).catch(handleErr)
  }, [token, projects.length, handleErr])

  useEffect(() => {
    if (!token || !effectiveProject) return
    load().catch(handleErr)
  }, [token, effectiveProject, load, handleErr])

  // Live updates by polling the event log. `EventSource` cannot set an
  // Authorization header, which is why this is a poll and not the SSE stream.
  useEffect(() => {
    if (!token || !effectiveProject) return
    const id = window.setInterval(() => {
      getEvents(token, cursor)
        .then((page) => {
          if (page.cursor != null) setCursor(page.cursor)
          setConn('live')
          if ((page.items?.length ?? 0) > 0) void load()
        })
        // A failed poll is not an error to shout about, but the board must stop
        // claiming to be live — silently stale is the failure worth surfacing.
        .catch(() => setConn('reconnecting'))
    }, POLL_MS)
    return () => window.clearInterval(id)
  }, [token, effectiveProject, cursor, load])

  // Open questions per ticket — what the detail drawer's callout counts.
  useEffect(() => {
    if (!token || !effectiveProject) return
    listQuestions(token, { project: effectiveProject, status: 'open' })
      .then(setQuestions)
      .catch(() => setQuestions([]))
  }, [token, effectiveProject, tickets])

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
    if (tagFilter) out = out.filter((x) => (x.tags ?? []).includes(tagFilter))
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
  }, [tickets, ticketFilter, tagFilter, epicFilter, labelFilter, showArchived, mineOnly, me, index])

  const states = useMemo(() => workflow?.states?.map((s) => s.id) ?? [], [workflow])
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
  // A tag is `kind:handle`. The KIND stays a <select> — a project has a handful
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
      setSelectedId(id)
      const known = tickets.find((x) => x.id === id) ?? null
      setDetail(known)
      if (token) getTicket(token, id).then(setDetail).catch(() => {})
    },
    [tickets, token],
  )

  const currentProject = projects.find((p) => p.id === effectiveProject) as
    | (Project & Record<string, unknown>)
    | undefined

  if (!token) {
    return (
      <TokenGate
        title="takomo · board"
        subtitle={t.gateSub}
        tokenLabel={t.gateTokenLabel}
        openLabel={t.gateOpen}
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
    <div className="flex h-screen flex-col overflow-hidden">
      <AppHeader
        onNavigate={navigate}
        current="board"
        nav={{ board: t.board, inbox: t.inbox, initiatives: t.initiatives, schedules: t.schedules }}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
        projects={projects.map((p) => ({ id: p.id }))}
        project={effectiveProject}
        onProject={(id) => {
          // An explicit pick DOES change the shared selection — that is a human
          // saying which project they mean, on every surface.
          setProject(id)
          saveProject(id)
          setTicketFilter('')
          setTagFilter('')
        }}
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
          }}
        />
        {/* The SAME control as the ticket filter, mounted again — see
            components/Typeahead.tsx. Two mount points, one implementation. */}
        <select
          id="tagkindsel"
          aria-label={t.tagsHdr}
          value={tagKind}
          onChange={(e) => {
            setTagKind(e.target.value)
            // The old value belongs to the old kind; keeping it would filter to
            // an empty board with no visible reason.
            setTagFilter('')
          }}
          className="bg-muted text-foreground border-border cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
        >
          <option value="">{t.allTags}</option>
          {tagKinds.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
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
          }}
        />
        <label className="text-muted-foreground flex items-center gap-1.5 text-[12px] font-[650]">
          <input
            type="checkbox"
            checked={groupByEpic}
            onChange={(e) => setGroupByEpic(e.target.checked)}
          />
          {t.groupEpic}
        </label>
        <label className="text-muted-foreground flex items-center gap-1.5 text-[12px] font-[650]">
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(e) => setShowArchived(e.target.checked)}
          />
          {t.archived}
        </label>
        {me.expertise.length > 0 && (
          <label className="text-muted-foreground flex items-center gap-1.5 text-[12px] font-[650]">
            <input
              type="checkbox"
              checked={mineOnly}
              onChange={(e) => setMineOnly(e.target.checked)}
            />
            {t.mine}
          </label>
        )}
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
        <span
          title={conn === 'live' ? t.live : conn === 'reconnecting' ? t.reconnecting : t.loading}
          className={cn(
            'size-2 rounded-full',
            conn === 'live' && 'bg-ok',
            conn === 'reconnecting' && 'bg-crit',
            (conn === 'idle' || conn === 'loading') && 'bg-muted-foreground',
          )}
        />
        <Button variant="outline" size="icon" title={t.settings} onClick={() => {
          setSettings(settingsFrom(currentProject))
          setOrigSettings(settingsFrom(currentProject))
          setSaved(false)
          setSaveErr('')
          setSettingsOpen(true)
        }}>
          ⚙
        </Button>
        <Button variant="outline" size="icon" title={t.refresh} onClick={() => void load()}>
          ↻
        </Button>
        <Button
          variant="outline"
          size="icon"
          title={t.signout}
          onClick={() => {
            saveToken('')
            setToken('')
          }}
        >
          ⎋
        </Button>
      </AppHeader>

      <main className="min-h-0 flex-1 overflow-x-auto p-3">
        {epicGroups ? (
          <div className="flex flex-col gap-4">
            {[...epicGroups.entries()].map(([epic, ts]) => (
              <div key={epic || '(none)'}>
                <div className="text-muted-foreground mb-2 px-1 text-[11.5px] font-[750] tracking-[0.06em] uppercase">
                  {epic ? (index[epic]?.title ?? epic) : t.noEpic}
                </div>
                <div className="flex gap-3">
                  {states.map((s) => (
                    <Column
                      key={s}
                      state={s}
                      tickets={ts.filter((x) => x.state === s)}
                      selectedId={selectedId}
                      labels={{ showMore: t.showMore, blocked: t.blockedN, fromSchedule: t.fromSchedule, notFulfilled: t.notFulfilled }}
                      isDone={workflow?.states?.find((w) => w.id === s)?.terminal}
                      onOpen={openTicket}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="flex h-full min-h-0 gap-3">
            {[...columns.entries()].map(([state, ts]) => (
              <Column
                key={state}
                state={state}
                tickets={ts}
                selectedId={selectedId}
                labels={{ showMore: t.showMore, blocked: t.blockedN, fromSchedule: t.fromSchedule, notFulfilled: t.notFulfilled }}
                isDone={workflow?.states?.find((w) => w.id === state)?.terminal}
                onOpen={openTicket}
              />
            ))}
          </div>
        )}
      </main>

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
          blocksRel: t.blocksRel,
          links: t.links,
          blockedN: t.blockedN,
          answeringResumes: t.answeringResumes,
          decisionRouted: t.decisionRouted,
          answerInInbox: t.answerInInbox,
          inConvN: t.inConvN,
          inConvSub: t.inConvSub,
          readThread: t.readThread,
          askHuman: t.askHuman,
          close: t.setClose,
          promotions: t.promotions,
          comments: t.comments,
          noComments: t.noComments,
          refLabel: t.refLabel,
          agoSep: t.agoSep,
        }}
        onClose={() => {
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
          blocking: t.blocking,
          advisory: t.advisory,
          blockingHint: t.answeringResumes,
          advisoryHint: t.decisionRouted,
          langHint: t.setLangHelp,
          ask: t.send,
          cancel: t.setCancel,
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
          close: t.setClose,
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

      <SettingsSheet
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        settings={settings}
        onChange={(patch) => setSettings((cur) => ({ ...cur, ...patch }))}
        readOnly={false}
        saving={saving}
        saved={saved}
        error={saveErr}
        onSave={() => {
          setSaving(true)
          setSaveErr('')
          saveProjectSettings(token, effectiveProject, settings, origSettings)
            .then((calls) => {
              // Nothing changed → close, do not claim a save that never
              // happened. "Saved." over an unchanged form is a small lie that
              // teaches the reader to distrust the message.
              if (calls === 0) {
                setSettingsOpen(false)
                return
              }
              setSaved(true)
              setOrigSettings(settings)
              return listProjects(token).then(setProjects)
            })
            .catch((e: Error) => setSaveErr(e.message))
            .finally(() => setSaving(false))
        }}
        labels={{
          title: t.setTitle,
          subtitle: t.setSub,
          langLabel: t.setLangLabel,
          langHelp: t.setLangHelp,
          langPh: t.setLangPh,
          styleLabel: t.setStyleLabel,
          styleHelp: t.setStyleHelp,
          stylePh: t.setStylePh,
          ttlLabel: t.setTtlLabel,
          ttlHelp: t.setTtlHelp,
          claimTtlLabel: t.setClaimTtlLabel,
          claimTtlHelp: t.setClaimTtlHelp,
          maxClaimTtlLabel: t.setMaxClaimTtlLabel,
          maxClaimTtlHelp: t.setMaxClaimTtlHelp,
          chars: t.setChars,
          over: t.setOver,
          save: t.setSave,
          saving: t.setSaving,
          savedMsg: t.setSaved,
          cancel: t.setCancel,
          readOnlyMsg: t.setReadOnly,
        }}
      />
    </div>
  )
}
