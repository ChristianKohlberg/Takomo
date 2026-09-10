import { useLiveRefresh } from '@/hooks/useLiveRefresh'
import { saveProject } from '@/lib/session'
import { useEffect, useId, useRef, useState } from 'react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { Locale } from '@/lib/i18n'
import { flattenSections, planSections, type PlanNode } from '@/lib/plan-sections'
import type { ApiErrorShape } from '@/lib/api'
import { getDocumentConversation, sendDocumentMessage, setDocumentPins, type DocumentContext, type DocumentAction, type DocumentConversationView, type DocumentRequest, type DocumentScope } from '@/lib/document-conversation'
import { DOCUMENT_CHAT } from './document-conversation-strings'

export interface DocumentIntent extends DocumentScope { action: DocumentAction; nonce: number; mode?: DocumentContext['mode']; quote?: DocumentContext['quote'] }
interface Props {
  token: string; project: string; map: string; lang: Locale; nodes: PlanNode[]; canAsk: boolean
  open: boolean; visible?: boolean; onOpenChange: (open: boolean) => void; restoreFocus: () => void
  intent: DocumentIntent; onNavigate?: (section: string) => void; onError?: (error: unknown) => void
}
export function DocumentConversation(props: Props) {
  return <Conversation key={`${props.token}:${props.map}`} {...props} />
}

function presetDraft(current: string, next: DocumentAction, t: (typeof DOCUMENT_CHAT)[Locale]) {
    const preset = (['discuss', 'grill', 'draft_tests', 'draft_questions'] as const).some(value => current === t[`${value}Prompt`])
    return !current.trim() || preset ? t[`${next}Prompt`] : current
  }

function Conversation({ token, project, map, lang, nodes, canAsk, open, visible = true, onOpenChange, restoreFocus, intent, onNavigate, onError }: Props) {
  const t = DOCUMENT_CHAT[lang]
  const [view, setView] = useState<DocumentConversationView | null>(null)
  const [action, setAction] = useState<DocumentAction>(intent.action)
  const [scope, setScope] = useState<DocumentScope>(intent)
  const [mode, setMode] = useState<DocumentContext['mode']>(intent.mode ?? (intent.whole_document ? 'whole_document' : 'selected'))
  const [quote, setQuote] = useState(intent.quote)
  const [pins, setPins] = useState<string[]>([])
  const [pinSaving, setPinSaving] = useState(false)
  const pinsDirty = useRef(false)
  const pinController = useRef<AbortController | null>(null)
  const [slashIndex, setSlashIndex] = useState(0)
  const [slashDismissed, setSlashDismissed] = useState(false)
  const [draft, setDraft] = useState<string>(t[`${intent.action}Prompt`])
  const [error, setError] = useState('')
  const [sending, setSending] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const [pendingRetry, setPendingRetry] = useState(false)
  const history = useRef<HTMLDivElement>(null)
  const composer = useRef<HTMLTextAreaElement>(null)
  const focusOnLoad = useRef(open)
  useEffect(() => { focusOnLoad.current = open }, [open])
  useEffect(() => { if (open && view && focusOnLoad.current && composer.current) { composer.current.focus({ preventScroll: true }); focusOnLoad.current = false } }, [open, view])
  const followTail = useRef(true)
  const submissionError = useRef(false)
  const inFlight = useRef(false)
  const generation = useRef(0)
  const request = useRef<DocumentRequest | null>(null)
  const postController = useRef<AbortController | null>(null)
  const lastIntent = useRef(intent)
  const onErrorRef = useRef(onError)
  useEffect(() => { onErrorRef.current = onError }, [onError])
  useEffect(() => () => { generation.current++; postController.current?.abort(); pinController.current?.abort() }, [])
  useEffect(() => {
    if (lastIntent.current === intent) return
    lastIntent.current = intent
    // Closing/reopening the sheet keeps an uncertain request intact, including
    // its original IDs and scope. A command must not silently replace it.
    if (inFlight.current || request.current) return
    setAction(intent.action); setScope(intent); setMode(intent.mode ?? (intent.whole_document ? 'whole_document' : 'selected')); setQuote(intent.quote); setDraft(current => presetDraft(current, intent.action, t))
  }, [intent, t])
  const sections = flattenSections(planSections(nodes))
  const id = useId()
  const tooLong = new TextEncoder().encode(draft.trim()).length > 8000
  const validScope = (mode !== 'selected' || scope.section_ids.length > 0 || pins.length > 0) && [...(mode === 'selected' ? scope.section_ids : []), ...pins, ...(quote ? [quote.section_id] : [])].every(section => nodes.some(node => node.id === section))
  const atTurnLimit = !!view && view.jobs.length >= (view.turn_limit ?? 100)
  const active = view?.jobs.find(job => job.status === 'queued' || job.status === 'running')
  const latest = view?.jobs.reduce<DocumentConversationView['jobs'][number] | undefined>((last, job) => !last || job.created_at >= last.created_at ? job : last, undefined)
  const locked = sending || !!active || pendingRetry || pinSaving

  useEffect(() => {
    if (open && followTail.current && history.current) history.current.scrollTop = history.current.scrollHeight
  }, [open, view?.messages.length])
  useLiveRefresh({
    token, project, scope: `${map}:${refresh}`, topics: ['agent'],
    enabled: open && visible, paused: sending, activeMs: active ? 1000 : false,
    onError: cause => {
      setError(cause instanceof Error ? cause.message : t.loadFailed)
      if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause)
    },
    load: async signal => {
      const epoch = generation.current
      const next = await getDocumentConversation(token, map, signal)
      if (signal.aborted || epoch !== generation.current || inFlight.current) return
      setView(next)
      if (!pinsDirty.current) setPins(next.pinned_section_ids ?? [])
      if (!submissionError.current) setError('')
    },
  })

  async function send(retry = false) {
    if (!canAsk || inFlight.current || !view) return
    if (!retry && (active || pendingRetry || pinSaving || atTurnLimit || !validScope || !draft.trim() || tooLong)) return
    if (retry && !request.current) return
    inFlight.current = true
    followTail.current = true
    const epoch = ++generation.current
    const controller = new AbortController()
    postController.current = controller
    const command = /^\/(grill|tests|questions)(?:\s+|$)([\s\S]*)$/.exec(draft.trim())
    const nextAction = command ? ({ grill: 'grill', tests: 'draft_tests', questions: 'draft_questions' } as const)[command[1] as 'grill' | 'tests' | 'questions'] : action
    if (!retry) request.current = { message: command ? command[2]!.trim() || t[`${nextAction}Prompt`] : draft.trim(), request_id: crypto.randomUUID(), action: nextAction, context: { mode, section_ids: mode === 'selected' ? [...scope.section_ids] : [], pinned_section_ids: [...pins], ...(quote ? { quote } : {}) } }
    if (!retry) setAction(nextAction)
    const pending = request.current!
    submissionError.current = false
    setSending(true); setError('')
    try {
      const next = await sendDocumentMessage(token, map, pending, controller.signal)
      if (controller.signal.aborted || epoch !== generation.current) return
      setView(next); setDraft(''); setQuote(undefined); request.current = null; setPendingRetry(false)
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
  const slashActions = ([['/grill', 'grill'], ['/tests', 'draft_tests'], ['/questions', 'draft_questions']] as const).filter(([command]) => command.startsWith(draft.trim()))
  const slashOpen = !slashDismissed && draft.startsWith('/') && !draft.includes(' ') && slashActions.length > 0
  function preset(next: DocumentAction) { if (locked) return; setAction(next); setDraft(t[`${next}Prompt`]); setSlashDismissed(true) }
  async function pin(section: string) {
    if (locked || !canAsk) return
    const next = pins.includes(section) ? pins.filter(id => id !== section) : [...pins, section]
    const controller = new AbortController(); pinController.current = controller
    pinsDirty.current = true; setPinSaving(true); setError('')
    try {
      await setDocumentPins(token, map, next, controller.signal)
      if (!controller.signal.aborted) setPins(next)
    } catch (cause) { if (!controller.signal.aborted) { setError(cause instanceof Error ? cause.message : t.sendFailed); if ((cause as ApiErrorShape)?.auth) onErrorRef.current?.(cause) } }
    finally { if (!controller.signal.aborted) { pinsDirty.current = false; setPinSaving(false); setRefresh(value => value + 1) } }
  }
  function migrationNotice(job: DocumentConversationView['jobs'][number]) {
    return job.migration && <div role="status" className="mb-2 rounded border border-border-soft bg-muted p-2 text-xs text-muted-foreground">
      <p>{t.migrated.replace('{n}', String(job.migration.retained_turns))}</p>
      {job.migration.omitted_turns > 0 && <p className="mt-1">{t.omitted.replace('{n}', String(job.migration.omitted_turns))}</p>}
    </div>
  }
  return <aside hidden={!open} aria-label={t.title} className={`${open ? 'flex' : 'hidden'} min-h-0 flex-1 flex-col border-l border-border-soft bg-card`} onKeyDown={event => {
    if (event.key === 'Escape' && !event.defaultPrevented) { event.stopPropagation(); onOpenChange(false); restoreFocus() }
  }}>
      <header className="flex shrink-0 items-start gap-2 border-b border-border-soft p-3">
        <div className="min-w-0 flex-1"><h2 className="font-semibold">{t.title}</h2><p className="text-xs text-muted-foreground">{t.description}</p></div>
        <Button variant="ghost" size="sm" aria-label={t.close} onClick={() => { onOpenChange(false); restoreFocus() }}>×</Button>
      </header>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">
        {!view && !error && <p role="status">{t.loading}</p>}
        {view && <>
          <div ref={history} role="log" aria-label={t.title} className="min-h-24 flex-1 space-y-4 overflow-y-auto [overflow-wrap:anywhere]"
            onScroll={event => { const el = event.currentTarget; followTail.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 48 }}>
            {!view.messages.length && <div className="space-y-2"><p className="text-sm text-muted-foreground">{t.empty}</p>{canAsk && (['grill', 'draft_tests', 'draft_questions'] as const).map(value => <button type="button" disabled={locked} key={value} className="block rounded border border-border-soft px-3 py-2 text-left text-sm hover:bg-muted" onClick={() => preset(value)}>{t[value]}</button>)}</div>}
            {view.messages.map(message => {
              const job = view.jobs.find(item => item.id === message.job_id)
              return <article key={message.id} className="min-w-0">
                {message.role === 'assistant' && job && migrationNotice(job)}
                <p className="mb-1 text-xs font-semibold text-muted-foreground">{message.role === 'assistant' ? 'Codex' : t.member}</p>
                {message.role === 'user' && job && <div className="mb-2 space-y-1"><p className="text-xs">{t[job.action]}</p>{job.context?.mode === 'automatic' ? <p className="text-xs text-muted-foreground">{t.automatic}</p> : contextChips(job, true)}{job.context?.quote && <blockquote className="max-h-24 overflow-y-auto border-l-2 pl-2 text-xs">{job.context.quote.text}</blockquote>}</div>}
                <div onClick={event => {
                  const anchor = (event.target as Element).closest('a'); if (!anchor) return
                  const url = new URL(anchor.href, window.location.href); const id = url.searchParams.get('takomo-section')
                  if (url.origin === window.location.origin && id && job?.sources?.some(source => source.section_id === id)) { event.preventDefault(); onNavigate?.(id) }
                }}><Markdown text={message.body.replace(/\]\(takomo-section:([^\s)]+)\)/g, (match, section: string) => job?.sources?.some(source => source.section_id === section) ? `](${window.location.origin}${window.location.pathname}?takomo-section=${encodeURIComponent(section)})` : match)} diagramAccess={{ token, project }} className="min-w-0 overflow-x-auto text-sm" /></div>
                {message.role === 'assistant' && job && <div className="mt-2 space-y-1 text-xs">
                  <p className="text-muted-foreground">{t.used} · {job.sources?.length ?? job.section_count}{job.coverage ? ` · ${t.readCoverage.replace('{n}', String(job.coverage.read_section_ids.length)).replace('{total}', String(job.coverage.total_sections))}` : ''}{job.coverage && !job.coverage.complete ? ` · ${t.incomplete}` : ''}</p>
                  <div className="flex max-h-32 flex-wrap gap-1 overflow-y-auto">{(job.sources ?? job.sections?.map(section => ({ section_id: section.id, title: section.title, version: job.source_revision ?? '' })) ?? []).map(source => <button type="button" key={source.section_id} disabled={!nodes.some(node => node.id === source.section_id)} onClick={() => onNavigate?.(source.section_id)} title={`${t.version}: ${source.version}`} className="rounded border border-border-soft px-2 py-1 text-left hover:bg-muted disabled:opacity-50">{source.title || source.section_id}</button>)}</div>
                  {!!job.sources?.length && <details><summary className="cursor-pointer text-muted-foreground">{t.versions}</summary>{job.sources.map(source => <p key={source.section_id} className="break-all font-mono">{source.title || source.section_id}: {source.version}</p>)}</details>}
                </div>}
              </article>
            })}
          </div>
          {view.jobs.filter(job => job.migration && !view.messages.some(message => message.role === 'assistant' && message.job_id === job.id)).map(job => <div key={job.id}>{migrationNotice(job)}</div>)}
          {active && <div className="my-3 space-y-2 rounded border border-border-soft bg-muted/30 p-3 text-sm">
            <p role="status">{active.status === 'running' ? t.running : t.queued}</p>
            <p className="text-xs text-muted-foreground">{lang === 'de' ? 'Angefragt am' : 'Requested at'} <time dateTime={new Date(active.created_at).toISOString()}>{new Date(active.created_at).toLocaleString(lang)}</time></p>
            {active.status === 'queued' && <p className="text-xs text-muted-foreground">{lang === 'de' ? 'Die Verfügbarkeit des Agentendienstes ist hier nicht bekannt. Die Anfrage bleibt in der Warteschlange; erneutes Senden ist nicht nötig.' : 'Agent service availability is not reported here. Your request stays queued; you do not need to send it again.'}</p>}
            <div className="flex flex-wrap items-center gap-3 text-xs"><button type="button" className="underline" onClick={() => setRefresh(value => value + 1)}>{lang === 'de' ? 'Status aktualisieren' : 'Refresh status'}</button><a className="underline" href={`/agent-queues?project=${encodeURIComponent(project)}`} onClick={() => saveProject(project)}>{lang === 'de' ? 'Agent-Warteschlange öffnen' : 'Open agent queue'}</a></div>
          </div>}
          {!active && latest?.status === 'failed' && <p role="alert" className="my-3 text-sm text-destructive">{t.failed} {latest.error}</p>}
          {atTurnLimit && !pendingRetry ? <p className="mt-3 text-xs text-muted-foreground">{t.turnLimit}</p> : canAsk ? <form className="mt-4 shrink-0 space-y-3 border-t border-border-soft pt-3" onSubmit={event => { event.preventDefault(); void send() }}>
            <fieldset disabled={locked} className="space-y-2">
              <legend className="mb-2 text-xs font-semibold">{t.scope}</legend>
              <div className="flex flex-wrap gap-3 text-sm">
                <label className="flex items-center gap-2"><input type="radio" name={`${id}-scope`} checked={mode === 'automatic'} onChange={() => { setMode('automatic'); setQuote(undefined) }} />{t.automatic}</label>
                <label className="flex items-center gap-2"><input type="radio" name={`${id}-scope`} checked={mode === 'whole_document'} onChange={() => { setMode('whole_document'); setQuote(undefined); setScope({ whole_document: true, section_ids: [] }) }} />{t.whole}</label>
                <label className="flex items-center gap-2"><input type="radio" name={`${id}-scope`} checked={mode === 'selected'} onChange={() => { setMode('selected'); setScope({ whole_document: false, section_ids: scope.section_ids }) }} />{t.mine}</label>
              </div>
              {mode === 'selected' && <div className="max-h-36 space-y-1 overflow-y-auto rounded border border-border-soft p-2">
                {sections.map(node => <label key={node.key} style={{ paddingInlineStart: `${Math.min(node.depth, 6) * 12}px` }} className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-1" checked={scope.section_ids.includes(node.key)} onChange={event => setScope({ whole_document: false, section_ids: event.target.checked ? [...scope.section_ids, node.key] : scope.section_ids.filter(section => section !== node.key) })} /><span className="min-w-0 break-words"><span aria-hidden="true" className="mr-1 text-muted-foreground">{node.number}</span>{node.title || t.untitled}</span></label>)}
              </div>}
              {mode === 'selected' && <p className="text-xs text-muted-foreground">{t.exact}</p>}
              {mode === 'automatic' ? <p className="text-xs text-muted-foreground">{t.automaticHint}</p> : contextChips({ ...scope, whole_document: mode === 'whole_document' })}
              <details><summary className="cursor-pointer text-xs">{t.pins} ({pins.length})</summary><div className="max-h-32 space-y-1 overflow-y-auto pt-1">{sections.map(section => <label key={section.key} className="flex gap-2 text-xs"><input type="checkbox" checked={pins.includes(section.key)} onChange={() => void pin(section.key)} />{section.number} {section.title || t.untitled}</label>)}</div></details>
              {pins.filter(section => !nodes.some(node => node.id === section)).map(section => <button type="button" key={section} onClick={() => void pin(section)} className="text-xs underline">{t.unpin}: {t.removed} ({section})</button>)}
              {quote && <div className="rounded border border-border-soft p-2 text-xs"><p>{t.quote}</p><blockquote className="max-h-24 overflow-y-auto whitespace-pre-wrap">{quote.text}</blockquote><button type="button" className="underline" onClick={() => setQuote(undefined)}>{t.removeQuote}</button></div>}
              {!validScope && <p className="text-xs text-destructive">{t.choose}</p>}
              <label className="flex items-center gap-2 text-sm">{t.action}<select className="min-w-0 rounded border border-border-soft bg-background p-1" value={action} onChange={event => { const next = event.target.value as DocumentAction; setAction(next); setDraft(current => presetDraft(current, next, t)) }}>
                {(['discuss', 'grill', 'draft_tests', 'draft_questions'] as const).map(value => <option key={value} value={value}>{t[value]}</option>)}
              </select></label>
            </fieldset>
            <Textarea ref={composer} aria-label={t.prompt} aria-autocomplete="list" aria-controls={slashOpen ? `${id}-slash` : undefined} aria-activedescendant={slashOpen ? `${id}-slash-${slashIndex}` : undefined} value={draft} onChange={event => { setDraft(event.target.value); setSlashIndex(0); setSlashDismissed(false) }} placeholder={t.slashHint} onKeyDown={event => {
              if (!slashOpen) return
              if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setSlashDismissed(true) }
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setSlashIndex(value => (value + (event.key === 'ArrowDown' ? 1 : -1) + slashActions.length) % slashActions.length) }
              if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); preset(slashActions[Math.min(slashIndex, slashActions.length - 1)]![1]) }
            }} disabled={locked} maxLength={8000} rows={3} aria-invalid={tooLong} />
            {slashOpen && <div id={`${id}-slash`} role="listbox" aria-label={t.commands} className="max-h-40 overflow-y-auto rounded border border-border-soft">{slashActions.map(([command, next], index) => <button id={`${id}-slash-${index}`} key={command} type="button" role="option" aria-selected={index === slashIndex} className="block w-full p-2 text-left text-xs aria-selected:bg-muted" onClick={() => preset(next)}><strong>{command} · {t[next]}</strong><span className="block text-muted-foreground">{t[`${next}Prompt`]}</span></button>)}</div>}
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
  </aside>
}
