import { useEffect, useId, useRef, useState } from 'react'
import { MessageCircle } from 'lucide-react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { defineStrings, type Locale } from '@/lib/i18n'
import { getSectionConversation, sendSectionMessage, type SectionConversationView } from '@/lib/section-conversation'
import type { ApiErrorShape } from '@/lib/api'

const STR = defineStrings({
  en: {
    open: 'Grill this section', close: 'Hide conversation', heading: 'Section conversation · Codex',
    description: 'Shared with this project. Codex can discuss this section; it cannot edit the specification.',
    empty: 'Ask Codex to challenge assumptions, find ambiguities, and ask focused questions about this section.',
    start: 'Start grilling', initial: 'Challenge the assumptions and ambiguities in this section. Ask me focused questions, starting with the most important one.',
    loading: 'Loading conversation…', queued: 'Queued · waiting for the agent service', running: 'Working…',
    failed: 'The agent run failed.', loadFailed: 'Could not load the conversation.', sendFailed: 'Could not confirm your message was sent. Retry to check the same request.',
    send: 'Send', sending: 'Sending…', prompt: 'Your reply or question', you: 'Project member', retry: 'Retry',
    messageTooLong: 'Your message exceeds 8,000 UTF-8 bytes. Please shorten it.',
    turnLimit: 'This conversation has reached its turn limit. Its history remains available.',
    needPermission: "Asking Codex requires 'human' and 'write' permissions.",
  },
  de: {
    open: 'Abschnitt hinterfragen', close: 'Gespräch ausblenden', heading: 'Abschnittsgespräch · Codex',
    description: 'Im Projekt geteilt. Codex kann diesen Abschnitt besprechen, aber die Spezifikation nicht bearbeiten.',
    empty: 'Lass Codex Annahmen hinterfragen, Unklarheiten finden und gezielte Fragen zu diesem Abschnitt stellen.',
    start: 'Hinterfragen starten', initial: 'Hinterfrage die Annahmen und Unklarheiten in diesem Abschnitt. Stelle mir gezielte Fragen und beginne mit der wichtigsten.',
    loading: 'Gespräch wird geladen…', queued: 'In der Warteschlange · wartet auf den Agentendienst', running: 'In Arbeit…',
    failed: 'Der Agentenlauf ist fehlgeschlagen.', loadFailed: 'Das Gespräch konnte nicht geladen werden.', sendFailed: 'Das Senden konnte nicht bestätigt werden. Erneut versuchen prüft dieselbe Anfrage.',
    send: 'Senden', sending: 'Wird gesendet…', prompt: 'Deine Antwort oder Frage', you: 'Projektmitglied', retry: 'Erneut versuchen',
    messageTooLong: 'Deine Nachricht überschreitet 8.000 UTF-8-Bytes. Bitte kürze sie.',
    turnLimit: 'Dieses Gespräch hat sein Nachrichtenlimit erreicht. Der Verlauf bleibt verfügbar.',
    needPermission: "Für Codex werden die Berechtigungen 'human' und 'write' benötigt.",
  },
})

interface Props {
  token: string
  map: string
  node: string
  lang: Locale
  canAsk: boolean
  onError?: (error: unknown) => void
}

// Key the entire local lifecycle to its authorization and section. A late response
// from another section (or a signed-out viewer) must never enter this conversation.
export function SectionConversation(props: Props) {
  return <Conversation key={`${props.token}:${props.map}:${props.node}`} {...props} />
}

function Conversation({ token, map, node, lang, canAsk, onError }: Props) {
  const t = STR[lang]
  const storageKey = `takomo.section-conversation.${map}.${node}`
  const [open, setOpen] = useState(() => {
    try { return sessionStorage.getItem(storageKey) === 'open' } catch { return false }
  })
  const [view, setView] = useState<SectionConversationView | null>(null)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState('')
  const [sending, setSending] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const [pendingRetry, setPendingRetry] = useState(false)
  const history = useRef<HTMLDivElement>(null)
  const followTail = useRef(true)
  const submissionError = useRef(false)
  const inFlight = useRef(false)
  const generation = useRef(0)
  const request = useRef<{ message: string; id: string } | null>(null)
  const postController = useRef<AbortController | null>(null)
  const onErrorRef = useRef(onError)
  useEffect(() => { onErrorRef.current = onError }, [onError])
  useEffect(() => () => { generation.current++; postController.current?.abort() }, [])
  const id = useId()
  const tooLong = new TextEncoder().encode(draft.trim()).length > 8000
  const atTurnLimit = !!view && view.jobs.length >= (view.turn_limit ?? 100)
  const active = view?.jobs.find((job) => job.status === 'queued' || job.status === 'running')
  const latest = view?.jobs.reduce<SectionConversationView['jobs'][number] | undefined>(
    (last, job) => !last || job.created_at >= last.created_at ? job : last, undefined,
  )

  // Markdown children finish rendering before this effect reads their height.
  // Follow new replies unless the reader deliberately moved up the history.
  useEffect(() => {
    if (open && followTail.current && history.current) {
      history.current.scrollTop = history.current.scrollHeight
    }
  }, [open, view?.messages.length])

  useEffect(() => {
    if (!open || sending) return
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout> | undefined
    async function load() {
      const epoch = generation.current
      try {
        const next = await getSectionConversation(token, map, node, controller.signal)
        if (controller.signal.aborted || epoch !== generation.current || inFlight.current) return
        setView(next)
        // A failed POST may have committed before its connection was lost.
        // Keep its retry action until the exact request is acknowledged.
        if (!submissionError.current) setError('')
        timer = setTimeout(load, next.jobs.some((job) => job.status === 'queued' || job.status === 'running') ? 1000 : 4000)
      } catch (cause) {
        if (controller.signal.aborted || epoch !== generation.current) return
        setError(cause instanceof Error ? cause.message : t.loadFailed)
        if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause)
        timer = setTimeout(load, 4000)
      }
    }
    void load()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, sending, refresh, token, map, node, t.loadFailed])

  function toggle() {
    const next = !open
    if (next) followTail.current = true
    setOpen(next)
    try {
      if (next) sessionStorage.setItem(storageKey, 'open')
      else sessionStorage.removeItem(storageKey)
    } catch { /* Storage is optional; the conversation itself lives on the server. */ }
  }

  async function send(message: string, retry = false) {
    if (!canAsk || inFlight.current || (!retry && (active || pendingRetry || atTurnLimit)) || !view || !message.trim() || new TextEncoder().encode(message.trim()).length > 8000) return
    inFlight.current = true
    followTail.current = true
    const epoch = ++generation.current
    const controller = new AbortController()
    postController.current = controller
    if (!retry) request.current = { message: message.trim(), id: crypto.randomUUID() }
    const pending = request.current!
    submissionError.current = false
    setSending(true)
    setError('')
    try {
      const next = await sendSectionMessage(token, map, node, pending.message, pending.id, controller.signal)
      if (controller.signal.aborted || epoch !== generation.current) return
      setView(next)
      setDraft('')
      request.current = null
      setPendingRetry(false)
    } catch (cause) {
      if (controller.signal.aborted || epoch !== generation.current) return
      submissionError.current = true
      const status = (cause as ApiErrorShape)?.status
      // A rejected request did not enqueue work. Keep the draft editable;
      // only uncertain delivery needs the exact same request ID retried.
      const uncertain = !status || status >= 500
      setPendingRetry(uncertain)
      if (!uncertain) request.current = null
      setError(`${uncertain ? t.sendFailed + ' ' : ''}${cause instanceof Error ? cause.message : t.sendFailed}`)
      if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause)
    } finally {
      if (!controller.signal.aborted && epoch === generation.current) {
        inFlight.current = false
        setSending(false)
      }
    }
  }

  return (
    <div className="mt-3 min-w-0">
      <button type="button" onClick={toggle} aria-expanded={open} aria-controls={id}
        className="text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 text-xs">
        <MessageCircle className="size-3.5" />{open ? t.close : t.open}
      </button>
      {open && (
        <div id={id} role="region" aria-label={t.heading} className="border-border-soft bg-muted/25 mt-3 min-w-0 rounded-md border p-4">
          <h3 className="text-sm font-semibold">{t.heading}</h3>
          <p className="text-muted-foreground mt-1 text-xs">{t.description}</p>
          {!view && !error && <p role="status" className="mt-3 text-sm">{t.loading}</p>}
          {view && (
            <>
              <div ref={history} role="log" className="my-4 max-h-[420px] space-y-4 overflow-y-auto [overflow-wrap:anywhere]"
                onScroll={(event) => {
                  const element = event.currentTarget
                  followTail.current = element.scrollHeight - element.scrollTop - element.clientHeight <= 48
                }}>
                {view.messages.length === 0 && <p className="text-muted-foreground text-sm">{t.empty}</p>}
                {view.messages.map((message) => (
                  <article key={message.id} className="min-w-0">
                    <p className="text-muted-foreground mb-1 text-xs font-semibold">{message.role === 'assistant' ? 'Codex' : t.you}</p>
                    <Markdown text={message.body} className="min-w-0 overflow-x-auto text-sm" />
                  </article>
                ))}
              </div>
              {active && <p role="status" className="mb-3 text-sm text-muted-foreground">{active.status === 'running' ? t.running : t.queued}</p>}
              {!active && latest?.status === 'failed' && <p role="alert" className="mb-3 text-sm text-destructive">{t.failed} {latest.error}</p>}
              {atTurnLimit && !pendingRetry ? <p className="text-muted-foreground text-xs">{t.turnLimit}</p> : canAsk ? (
                <form onSubmit={(event) => { event.preventDefault(); void send(draft) }} className="space-y-2">
                  <Textarea aria-label={t.prompt} placeholder={t.prompt} value={draft} onChange={(event) => setDraft(event.target.value)}
                    disabled={sending || !!active || pendingRetry} maxLength={8000} rows={3}
                    aria-invalid={tooLong} aria-describedby={tooLong ? `${id}-length` : undefined} />
                  {tooLong && <p id={`${id}-length`} role="alert" className="text-destructive text-xs">{t.messageTooLong}</p>}
                  <div className="flex flex-wrap gap-2">
                    {view.messages.length === 0 && <Button type="button" variant="outline" size="sm" disabled={sending || !!active || pendingRetry} onClick={() => void send(t.initial)}>{t.start}</Button>}
                    <Button type="submit" size="sm" disabled={!draft.trim() || tooLong || sending || !!active || pendingRetry}>{sending ? t.sending : t.send}</Button>
                  </div>
                </form>
              ) : <p className="text-muted-foreground text-xs">{t.needPermission}</p>}
            </>
          )}
          {error && <div role="alert" className="mt-3 text-sm text-destructive">
            <p>{error}</p>
            <Button type="button" variant="outline" size="sm" className="mt-2" disabled={sending} onClick={() => {
              if (request.current) void send(request.current.message, true)
              else { submissionError.current = false; setRefresh((value) => value + 1) }
            }}>{t.retry}</Button>
          </div>}
        </div>
      )}
    </div>
  )
}
