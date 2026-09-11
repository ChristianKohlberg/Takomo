import { useLiveRefresh } from '@/hooks/useLiveRefresh'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import type { ApiErrorShape } from '@/lib/api'
import { flattenSections, planSections, type PlanNode } from '@/lib/plan-sections'
import { specificationLink } from '@/lib/specification-url'
import { addTicketDocumentLink, changeTicketDocumentLink, classifyTicketDocument, documentSections, getTicketDocumentLinks, type DocumentReference, type TicketDocumentLinks as Links } from '@/lib/ticket-document-links'
import { DOCUMENT_LINKS } from './document-link-strings'
interface Props { token: string; project: string; ticket: string; lang: Locale; canWrite: boolean; onChanged?: () => void; onError?: (error: unknown) => void }
export function TicketDocumentLinks(props: Props) { return <Panel key={`${props.token}:${props.project}:${props.ticket}`} {...props} /> }
function Panel({ token, project, ticket, lang, canWrite, onChanged, onError }: Props) {
  const t = DOCUMENT_LINKS[lang]
  const [data, setData] = useState<Links | null>(null)
  const [sections, setSections] = useState<PlanNode[]>([])
  const [selected, setSelected] = useState('')
  const [replacing, setReplacing] = useState<string | null>(null)
  const [primary, setPrimary] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const picker = useRef<HTMLSelectElement>(null)
  const write = useRef<AbortController | null>(null)
  const pendingRequest = useRef<string | null>(null)
  const callbacks = useRef({ onChanged, onError }); callbacks.current = { onChanged, onError }
  useEffect(() => () => write.current?.abort(), [])
  useEffect(() => {
    const controller = new AbortController()
    documentSections(token, project, controller.signal).then(nodes => { if (!controller.signal.aborted) setSections(nodes) }).catch(cause => { if (!controller.signal.aborted) setError(cause instanceof Error ? cause.message : t.error) })
    return () => controller.abort()
  }, [token, project, refresh, t.error])
  useLiveRefresh({
    token, project, scope: `${ticket}:${refresh}`, topics: ['tickets', 'agent', 'document'], paused: busy,
    activeMs: ['queued', 'running'].includes(data?.classification?.status ?? '') ? 1500 : false,
    onError: cause => {
      setError(cause instanceof Error ? cause.message : t.error)
      if ((cause as ApiErrorShape)?.auth) callbacks.current.onError?.(cause)
    },
    load: async signal => {
      const result = await getTicketDocumentLinks(token, ticket, signal)
      if (!signal.aborted) setData(result)
    },
  })
  async function mutate(operation: (signal: AbortSignal) => Promise<unknown>) {
    if (write.current || !canWrite) return
    const controller = new AbortController(); write.current = controller; setBusy(true); setError('')
    try {
      await operation(controller.signal)
      if (controller.signal.aborted) return
      callbacks.current.onChanged?.(); setRefresh(value => value + 1)
    } catch (cause) {
      if (controller.signal.aborted) return
      setError(cause instanceof Error ? cause.message : t.error)
      if ((cause as ApiErrorShape)?.auth) callbacks.current.onError?.(cause)
    } finally { if (!controller.signal.aborted) { write.current = null; setBusy(false) } }
  }
  const accepted = data?.links.filter(link => link.state === 'accepted') ?? []
  const suggestions = data?.links.filter(link => link.state === 'suggested') ?? []
  const removed = data?.links.filter(link => link.state === 'removed') ?? []
  function date(value: string | number) {
    const parsed = new Date(typeof value === 'number' || /^\d+$/.test(value) ? Number(value) : value)
    return Number.isNaN(parsed.getTime()) ? String(value) : new Intl.DateTimeFormat(lang, { dateStyle: 'medium', timeStyle: 'short' }).format(parsed)
  }
  function linkView(link: DocumentReference) {
    return <div className="min-w-0 space-y-1 rounded border border-border-soft p-2" key={link.id}>
      <div className="flex flex-wrap items-center gap-2 text-sm">{link.missing ? <span>{link.title || link.section_id}</span> : <a href={specificationLink(project, 'document', link.section_id)} className="text-primary underline">{link.title || link.section_id}</a>}{link.primary && <span className="rounded bg-muted px-1 text-xs">{t.primary}</span>}</div>
      <p className="text-xs text-muted-foreground">{t[link.provenance]} · {t[link.relation]}</p>
      {(link.missing || link.stale) && <p className="text-xs text-amber-700 dark:text-amber-300">{link.missing ? t.missing : t.stale}</p>}
      {link.reason && <p className="text-xs">{link.reason}</p>}
      {link.quote && <blockquote className="max-h-24 overflow-y-auto border-l-2 pl-2 text-xs whitespace-pre-wrap">{link.quote}</blockquote>}
      <details className="text-xs text-muted-foreground"><summary className="cursor-pointer">{t.version}</summary><p className="break-all">{link.section_version}</p><p>{link.created_by} · {date(link.created_at)}</p>{link.captured_title && link.captured_title !== link.title && <p>{t.captured}: {link.captured_title}</p>}{link.reviewed_by && <p>{t.reviewed}: {link.reviewed_by}{link.reviewed_at ? ` · ${date(link.reviewed_at)}` : ''}</p>}</details>
      {canWrite && link.state !== 'removed' && <div className="flex flex-wrap gap-2">
        {link.state === 'suggested' && <><Button size="sm" variant="outline" disabled={busy || link.missing || link.stale} onClick={() => void mutate(signal => changeTicketDocumentLink(token, ticket, link.id, { state: 'accepted' }, signal))}>{t.accept}</Button><Button size="sm" variant="ghost" disabled={busy} onClick={() => { setReplacing(link.id); setSelected(''); picker.current?.focus() }}>{t.change}</Button></>}
        {link.state === 'accepted' && <Button size="sm" variant="ghost" disabled={busy || link.missing} onClick={() => void mutate(signal => changeTicketDocumentLink(token, ticket, link.id, { state: 'accepted', primary: !link.primary }, signal))}>{link.primary ? t.clearPrimary : t.makePrimary}</Button>}
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void mutate(signal => changeTicketDocumentLink(token, ticket, link.id, { state: 'removed' }, signal))}>{link.state === 'suggested' ? t.dismiss : t.remove}</Button>
      </div>}
    </div>
  }
  return <section aria-label={t.title} className="min-w-0 space-y-3 border-t border-border-soft pt-3">
    <h3 className="text-sm font-semibold">{t.title}</h3>
    {!data && !error && <p role="status" className="text-sm">{t.loading}</p>}
    {data && <>
      {accepted.length ? <div className="space-y-2" aria-label={t.accepted}>{accepted.map(linkView)}</div> : <p className="text-sm text-muted-foreground">{t.none}</p>}
      {suggestions.length > 0 && <div className="space-y-2"><h4 className="text-sm font-medium">{t.suggestions}</h4>{suggestions.map(linkView)}</div>}
      {data.classification && <div role="status" className="text-xs text-muted-foreground">
        {data.classification.status === 'no_match' || data.classification.status === 'unavailable' ? <p>{t.noMatch}</p> : <p>{({ linked: t.linked, queued: t.queued, running: t.running, completed: t.completed, failed: t.failed, cancelled: t.cancelled, stale: t.stale } as Record<string, string>)[data.classification.status] ?? data.classification.status}</p>}
        {data.classification.no_match_reason && <p>{data.classification.no_match_reason}</p>}{data.classification.error && <p>{data.classification.error}</p>}{data.classification.ambiguity && <p>{typeof data.classification.ambiguity === 'string' ? data.classification.ambiguity : t.ambiguity}</p>}
      </div>}
      {canWrite ? <>
        {replacing && <p className="text-xs">{t.replacing}<button type="button" className="ml-2 underline" onClick={() => setReplacing(null)}>{t.cancel}</button></p>}
        <form className="flex flex-wrap items-center gap-2" onSubmit={event => { event.preventDefault(); if (selected) void mutate(async signal => { await addTicketDocumentLink(token, ticket, selected, primary, signal); if (replacing) await changeTicketDocumentLink(token, ticket, replacing, { state: 'removed' }, signal); if (!signal.aborted) { setReplacing(null); setSelected('') } }) }}>
          <select ref={picker} aria-label={t.select} className="min-w-0 max-w-full rounded border border-border-soft bg-background p-2 text-sm" value={selected} onChange={event => setSelected(event.target.value)} disabled={busy}><option value="">{t.select}</option>{flattenSections(planSections(sections)).map(section => <option key={section.key} value={section.key}>{section.number} {section.title || section.key}</option>)}</select>
          <label className="flex items-center gap-1 text-xs"><input type="checkbox" checked={primary} onChange={event => setPrimary(event.target.checked)} disabled={busy} />{t.primary}</label>
          <Button type="submit" size="sm" disabled={busy || !selected}>{t.add}</Button>
        </form>
        {!sections.length && <p className="text-xs text-muted-foreground">{t.noSections}</p>}
        <Button size="sm" variant="outline" disabled={busy || ['queued', 'running'].includes(data.classification?.status ?? '')} onClick={() => void mutate(async signal => {
          pendingRequest.current ??= crypto.randomUUID()
          await classifyTicketDocument(token, ticket, pendingRequest.current, signal); pendingRequest.current = null
        })}>{t.retry}</Button>
      </> : <p className="text-xs text-muted-foreground">{t.readOnly}</p>}
      {removed.length > 0 && <details><summary className="cursor-pointer text-xs">{t.history} ({removed.length})</summary><div className="mt-2 space-y-2">{removed.map(linkView)}</div></details>}
    </>}
    {error && <div role="alert" className="text-sm text-destructive"><p>{error}</p><Button variant="ghost" size="sm" disabled={busy} onClick={() => { setError(''); setRefresh(value => value + 1) }}>{t.refresh}</Button></div>}
  </section>
}
