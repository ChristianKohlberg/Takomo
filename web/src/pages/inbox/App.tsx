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
import { useNavigate } from 'react-router'
import { loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { AnswerLinkDialog } from '@/components/inbox/AnswerLinkDialog'
import { FolderRail } from '@/components/inbox/FolderRail'
import { QuestionRow } from '@/components/inbox/QuestionRow'
import { ReadingPane } from '@/components/inbox/ReadingPane'
import { Typeahead } from '@/components/Typeahead'
import { UndoSnackbar } from '@/components/inbox/UndoSnackbar'
import { useUndoQueue } from '@/hooks/useUndoQueue'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { answerPayloadFor, displayValue, type Draft } from '@/lib/answers'
import { undoInto } from '@/lib/undo-queue'
import {
  FOLDERS,
  answerQuestion,
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
const POLL_MS = 5000

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => loadProject())
  const [gateError, setGateError] = useState('')

  const [me, setMe] = useState({ actor: '', scopes: [] as string[] })
  const [projects, setProjects] = useState<Project[]>([])
  const [questions, setQuestions] = useState<Question[]>([])
  const [folder, setFolder] = useState<Folder>('open')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [thread, setThread] = useState<ThreadMessage[]>([])
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const [link, setLink] = useState<AnswerLink | null>(null)
  const [tickets, setTickets] = useState<TicketRef[]>([])
  const [ticket, setTicket] = useState('')

  const t = useMemo(() => pick(STR, lang), [lang])
  const canAnswer = me.scopes.includes('human')

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
        setMe({ actor: who.actor ?? '', scopes: who.scopes ?? [] })
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
    const id = window.setInterval(() => void fetchAll().catch(() => {}), POLL_MS)
    return () => window.clearInterval(id)
  }, [token, queue.pending.length, fetchAll])

  // `visible()` — everything the current filters admit. The folder split and the
  // counts both read from it, so a filtered-out question cannot be counted in a
  // folder it is not listed in.
  const visible = useMemo(
    () => (ticket ? questions.filter((q) => q.ticket === ticket) : questions),
    [questions, ticket],
  )
  const inFolder = useMemo(
    () => visible.filter((q) => q.status === folder),
    [visible, folder],
  )
  const counts = useMemo(() => {
    const c: Partial<Record<Folder, number>> = {}
    for (const f of FOLDERS) c[f] = visible.filter((q) => q.status === f).length
    return c
  }, [visible])

  const selected = useMemo(
    () => inFolder.find((q) => q.id === selectedId) ?? inFolder[0] ?? null,
    [inFolder, selectedId],
  )

  // Deep-linkable: #q=<id> opens that question, and selecting one writes it back
  // so the URL is shareable and bookmarkable.
  useEffect(() => {
    const m = /(?:^#|&)q=([^&]+)/.exec(window.location.hash || '')
    if (m?.[1]) setSelectedId(decodeURIComponent(m[1]))
  }, [])
  useEffect(() => {
    if (selected) window.location.hash = 'q=' + selected.id
  }, [selected])

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
        const i = inFolder.findIndex((q) => q.id === selected?.id)
        const next = e.key === 'j' ? i + 1 : i - 1
        if (next >= 0 && next < inFolder.length) setSelectedId(inFolder[next]!.id)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [inFolder, selected])

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
    const rest = inFolder.filter((x) => x.id !== q.id)
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
  }

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <AppHeader
        onNavigate={navigate}
        current="inbox"
        nav={{
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          schedules: t.schedules,
          settings: t.settings,
        }}
        badges={{ inbox: counts.open ?? 0 }}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
        projects={projects.map((p) => ({ id: p.id }))}
        project={project}
        allProjectsLabel={t.allProjects}
        onProject={(id) => {
          setProject(id)
          saveProject(id)
          setSelectedId(null)
          setTicket('')
        }}
      >
        <Typeahead
          id="tickpick"
          options={tickets}
          value={ticket}
          onChange={(id) => {
            setTicket(id)
            setSelectedId(null)
          }}
          labels={{
            all: t.allTickets,
            placeholder: t.taTicket,
            clear: t.taClear,
            noMatch: t.taNoMatch,
            count: t.taCount,
            count1: t.taCount1,
          }}
        />
        <span className="text-muted-foreground mr-1 hidden text-[11.5px] md:inline">{t.kbd}</span>
        <Button variant="outline" size="icon" title="Refresh" onClick={() => void fetchAll()}>
          ↻
        </Button>
        <Button variant="outline" size="icon" title="Sign out" onClick={signOut}>
          ⎋
        </Button>
      </AppHeader>

      <main className="grid min-h-0 flex-1 grid-cols-[180px_320px_1fr]">
        <FolderRail
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
          onSelect={(f) => {
            setFolder(f)
            setSelectedId(null)
          }}
        />

        <section className="bg-card border-r-border-soft min-h-0 overflow-y-auto border-r">
          {inFolder.length === 0 ? (
            <div className="text-muted-foreground px-6 py-14 text-center">
              <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.allClear}</div>
              <div className="text-[13px]">{t.allClearSub}</div>
            </div>
          ) : (
            inFolder.map((q) => (
              <QuestionRow
                key={q.id}
                question={q}
                selected={q.id === selected?.id}
                landed={queue.pending.some((p) => p.qid === q.id)}
                labels={{ advisory: t.advTag, askedBy: t.askedBy, waitingAgent: t.stallTag }}
                onSelect={setSelectedId}
              />
            ))
          )}
        </section>

        {selected && paneLabels ? (
          <ReadingPane
            key={selected.id}
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
    </div>
  )
}
