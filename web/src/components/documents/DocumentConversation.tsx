import { useEffect, useId, useRef, useState } from 'react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import type { Locale } from '@/lib/i18n'
import { flattenSections, planSections, type PlanNode } from '@/lib/plan-sections'
import type { ApiErrorShape } from '@/lib/api'
import { getDocumentConversation, sendDocumentMessage, type DocumentAction, type DocumentConversationView, type DocumentRequest, type DocumentScope } from '@/lib/document-conversation'
import { DOCUMENT_CHAT } from './document-conversation-strings'

export interface DocumentIntent extends DocumentScope { action: DocumentAction; nonce: number }
interface Props {
  token: string; project: string; map: string; lang: Locale; nodes: PlanNode[]; canAsk: boolean
  open: boolean; onOpenChange: (open: boolean) => void; restoreFocus: () => void
  intent: DocumentIntent; onError?: (error: unknown) => void
}
export function DocumentConversation(props: Props) {
  return <Conversation key={`${props.token}:${props.map}`} {...props} />
}

function presetDraft(current: string, next: DocumentAction, t: (typeof DOCUMENT_CHAT)[Locale]) {
    const preset = (['discuss', 'grill', 'draft_tests', 'draft_questions'] as const).some(value => current === t[`${value}Prompt`])
    return !current.trim() || preset ? t[`${next}Prompt`] : current
  }

function Conversation({ token, project, map, lang, nodes, canAsk, open, onOpenChange, restoreFocus, intent, onError }: Props) {
  const t = DOCUMENT_CHAT[lang]
  const [view, setView] = useState<DocumentConversationView | null>(null)
  const [action, setAction] = useState<DocumentAction>(intent.action)
  const [scope, setScope] = useState<DocumentScope>(intent)
  const [draft, setDraft] = useState<string>(t[`${intent.action}Prompt`])
  const [error, setError] = useState('')
  const [sending, setSending] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const [pendingRetry, setPendingRetry] = useState(false)
  const history = useRef<HTMLDivElement>(null)
  const followTail = useRef(true)
  const submissionError = useRef(false)
  const inFlight = useRef(false)
  const generation = useRef(0)
  const request = useRef<DocumentRequest | null>(null)
  const postController = useRef<AbortController | null>(null)
  const lastIntent = useRef(intent)
  const onErrorRef = useRef(onError)
  useEffect(() => { onErrorRef.current = onError }, [onError])
  useEffect(() => () => { generation.current++; postController.current?.abort() }, [])
  useEffect(() => {
    if (lastIntent.current === intent) return
    lastIntent.current = intent
    // Closing/reopening the sheet keeps an uncertain request intact, including
    // its original IDs and scope. A command must not silently replace it.
    if (inFlight.current || request.current) return
    setAction(intent.action); setScope(intent); setDraft(current => presetDraft(current, intent.action, t))
  }, [intent, t])
  const sections = flattenSections(planSections(nodes))
  const id = useId()
  const tooLong = new TextEncoder().encode(draft.trim()).length > 8000
  const validScope = scope.whole_document ? true : scope.section_ids.length > 0 && scope.section_ids.every(section => nodes.some(node => node.id === section))
  const atTurnLimit = !!view && view.jobs.length >= (view.turn_limit ?? 100)
  const active = view?.jobs.find(job => job.status === 'queued' || job.status === 'running')
  const latest = view?.jobs.reduce<DocumentConversationView['jobs'][number] | undefined>((last, job) => !last || job.created_at >= last.created_at ? job : last, undefined)
  const locked = sending || !!active || pendingRetry

  useEffect(() => {
    if (open && followTail.current && history.current) history.current.scrollTop = history.current.scrollHeight
  }, [open, view?.messages.length])
  useEffect(() => {
    if (!open || sending) return
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout> | undefined
    async function load() {
      const epoch = generation.current
      try {
        const next = await getDocumentConversation(token, map, controller.signal)
        if (controller.signal.aborted || epoch !== generation.current || inFlight.current) return
        setView(next)
        if (!submissionError.current) setError('')
        timer = setTimeout(load, next.jobs.some(job => job.status === 'queued' || job.status === 'running') ? 1000 : 4000)
      } catch (cause) {
        if (controller.signal.aborted || epoch !== generation.current) return
        setError(cause instanceof Error ? cause.message : t.loadFailed)
        if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause)
        timer = setTimeout(load, 4000)
      }
    }
    void load()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, sending, refresh, token, map, t.loadFailed])

  async function send(retry = false) {
    if (!canAsk || inFlight.current || !view) return
    if (!retry && (active || pendingRetry || atTurnLimit || !validScope || !draft.trim() || tooLong)) return
    if (retry && !request.current) return
    inFlight.current = true
    followTail.current = true
    const epoch = ++generation.current
    const controller = new AbortController()
    postController.current = controller
    if (!retry) request.current = { message: draft.trim(), request_id: crypto.randomUUID(), action, whole_document: scope.whole_document, section_ids: scope.whole_document ? [] : [...scope.section_ids] }
    const pending = request.current!
    submissionError.current = false
    setSending(true); setError('')
    try {
      const next = await sendDocumentMessage(token, map, pending, controller.signal)
      if (controller.signal.aborted || epoch !== generation.current) return
      setView(next); setDraft(''); request.current = null; setPendingRetry(false)
    } catch (cause) {
      if (controller.signal.aborted || epoch !== generation.current) return
      submissionError.current = true
      const status = (cause as ApiErrorShape)?.status
      const uncertain = !status || status >= 500
      setPendingRetry(uncertain)
      if (!uncertain) request.current = null
      setError(`${uncertain ? t.sendFailed + ' ' : ''}${cause instanceof Error ? cause.message : t.sendFailed}`)
      if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause)
    } finally {
      if (!controller.signal.aborted && epoch === generation.current) { inFlight.current = false; setSending(false) }
    }
  }
  function contextChips(context: DocumentScope & { section_count?: number; sections?: { id: string; title: string }[] }, historical = false) {
    const count = context.section_count ?? (context.whole_document ? nodes.length : context.section_ids.length)
    const label = t.selected.replace('{n}', String(count))
    const chips = context.section_ids.map(section => {
      const saved = context.sections?.find(item => item.id === section)
      const live = nodes.find(node => node.id === section)
      const title = historical ? saved?.title || section : live ? live.title || t.untitled : `${t.removed} (${section})`
      return <span key={section} title={section} className="max-w-full break-words rounded border border-border-soft px-2 py-1">{title}</span>
    })
    return <div className="min-w-0 text-xs" aria-label={t.scope}>
      <span className="inline-block rounded bg-muted px-2 py-1">{context.whole_document ? `${t.whole} · ${label}` : label}</span>
      {!context.whole_document && (chips.length > 4 ? <details className="mt-1"><summary className="cursor-pointer">{t.sections}</summary><div className="mt-1 flex max-h-24 flex-wrap gap-1 overflow-y-auto">{chips}</div></details> : <div className="mt-1 flex flex-wrap gap-1">{chips}</div>)}
    </div>
  }
  return <Sheet open={open} onOpenChange={onOpenChange}>
    <SheetContent className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-xl" onCloseAutoFocus={event => { event.preventDefault(); restoreFocus() }}>
      <SheetHeader className="border-b border-border-soft pr-12">
        <SheetTitle>{t.title}</SheetTitle><SheetDescription>{t.description}</SheetDescription>
      </SheetHeader>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">
        {!view && !error && <p role="status">{t.loading}</p>}
        {view && <>
          <div ref={history} role="log" aria-label={t.title} className="min-h-24 flex-1 space-y-4 overflow-y-auto [overflow-wrap:anywhere]"
            onScroll={event => { const el = event.currentTarget; followTail.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 48 }}>
            {!view.messages.length && <p className="text-sm text-muted-foreground">{t.empty}</p>}
            {view.messages.map(message => {
              const job = view.jobs.find(item => item.id === message.job_id)
              return <article key={message.id} className="min-w-0">
                <p className="mb-1 text-xs font-semibold text-muted-foreground">{message.role === 'assistant' ? 'Codex' : t.member}</p>
                {message.role === 'user' && job && <div className="mb-2 space-y-1"><p className="text-xs">{t[job.action]}</p>{contextChips(job, true)}</div>}
                <Markdown text={message.body} diagramAccess={{ token, project }} className="min-w-0 overflow-x-auto text-sm" />
              </article>
            })}
          </div>
          {active && <p role="status" className="my-3 text-sm text-muted-foreground">{active.status === 'running' ? t.running : t.queued}</p>}
          {!active && latest?.status === 'failed' && <p role="alert" className="my-3 text-sm text-destructive">{t.failed} {latest.error}</p>}
          {atTurnLimit && !pendingRetry ? <p className="mt-3 text-xs text-muted-foreground">{t.turnLimit}</p> : canAsk ? <form className="mt-4 shrink-0 space-y-3 border-t border-border-soft pt-3" onSubmit={event => { event.preventDefault(); void send() }}>
            <fieldset disabled={locked} className="space-y-2">
              <legend className="mb-2 text-xs font-semibold">{t.scope}</legend>
              <div className="flex flex-wrap gap-3 text-sm">
                <label className="flex items-center gap-2"><input type="radio" name={`${id}-scope`} checked={scope.whole_document} onChange={() => setScope({ whole_document: true, section_ids: [] })} />{t.whole}</label>
                <label className="flex items-center gap-2"><input type="radio" name={`${id}-scope`} checked={!scope.whole_document} onChange={() => setScope({ whole_document: false, section_ids: [] })} />{t.sections}</label>
              </div>
              {!scope.whole_document && <div className="max-h-36 space-y-1 overflow-y-auto rounded border border-border-soft p-2">
                {sections.map(node => <label key={node.key} style={{ paddingInlineStart: `${Math.min(node.depth, 6) * 12}px` }} className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-1" checked={scope.section_ids.includes(node.key)} onChange={event => setScope({ whole_document: false, section_ids: event.target.checked ? [...scope.section_ids, node.key] : scope.section_ids.filter(section => section !== node.key) })} /><span className="min-w-0 break-words"><span aria-hidden="true" className="mr-1 text-muted-foreground">{node.number}</span>{node.title || t.untitled}</span></label>)}
              </div>}
              {!scope.whole_document && <p className="text-xs text-muted-foreground">{t.exact}</p>}
              {contextChips(scope)}
              {!validScope && <p className="text-xs text-destructive">{t.choose}</p>}
              <label className="flex items-center gap-2 text-sm">{t.action}<select className="min-w-0 rounded border border-border-soft bg-background p-1" value={action} onChange={event => { const next = event.target.value as DocumentAction; setAction(next); setDraft(current => presetDraft(current, next, t)) }}>
                {(['discuss', 'grill', 'draft_tests', 'draft_questions'] as const).map(value => <option key={value} value={value}>{t[value]}</option>)}
              </select></label>
            </fieldset>
            <Textarea aria-label={t.prompt} value={draft} onChange={event => setDraft(event.target.value)} disabled={locked} maxLength={8000} rows={3} aria-invalid={tooLong} />
            {tooLong && <p role="alert" className="text-xs text-destructive">{t.tooLong}</p>}
            {pendingRetry && <p className="text-xs text-muted-foreground">{t.pending}</p>}
            <Button size="sm" type="submit" disabled={locked || !validScope || !draft.trim() || tooLong}>{sending ? t.sending : t.send}</Button>
          </form> : <p className="mt-4 text-xs text-muted-foreground">{t.permission}</p>}
        </>}
        {error && <div role="alert" className="mt-3 text-sm text-destructive"><p>{error}</p><Button variant="outline" size="sm" className="mt-2" disabled={sending || (!!request.current && !canAsk)} onClick={() => {
          if (request.current) void send(true)
          else { submissionError.current = false; setRefresh(value => value + 1) }
        }}>{t.retry}</Button></div>}
      </div>
    </SheetContent>
  </Sheet>
}
