import { useCallback, useEffect, useRef, useState } from 'react'
import { Sparkles, CheckCircle2, LoaderCircle, AlertTriangle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { Markdown } from '@/components/Markdown'
import { acceptOrganization, getOrganizer, requestOrganization, type OrganizerJob, type OrganizerView } from '@/lib/lane-organizer'
import { isAuthError } from '@/lib/session'
import { pick, type Locale } from '@/lib/i18n'
import { ORGANIZER_STR } from './organizer-strings'

type Props = { token: string; project: string; lang: Locale; canOrganize: boolean; onAuthError: () => void; onAccepted: () => void }
type Labels = typeof ORGANIZER_STR.en
const activeJob = (job: OrganizerJob) => job.status === 'queued' || job.status === 'running'
const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error)

export function Organizer(props: Props) {
  return <OrganizerSession key={`${props.token}:${props.project}`} {...props} />
}

function OrganizerSession({ token, project, lang, canOrganize, onAuthError, onAccepted }: Props) {
  const t = pick(ORGANIZER_STR, lang)
  const [view, setView] = useState<OrganizerView | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<boolean | null>(null)
  const [composing, setComposing] = useState(false)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState<'request' | 'accept' | null>(null)
  const [error, setError] = useState('')
  const [loadError, setLoadError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const request = useRef<{ request_id: string; message: string } | null>(null)
  const mounted = useRef(true)
  const reads = useRef<AbortController | null>(null)
  const viewRef = useRef(view)
  useEffect(() => { viewRef.current = view }, [view])
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const authError = useCallback((error: unknown) => {
    if (isAuthError(error)) { onAuthError(); return true }
    return false
  }, [onAuthError])
  useEffect(() => {
    const controller = new AbortController()
    reads.current = controller
    let timer: ReturnType<typeof setTimeout> | undefined
    async function load() {
      let poll = viewRef.current?.jobs.some(activeJob) ?? false
      try {
        const data = await getOrganizer(token, project, controller.signal)
        if (controller.signal.aborted) return
        setView(data); setLoadError(''); poll = data.jobs.some(activeJob)
      } catch (error) {
        if (controller.signal.aborted) return
        if (authError(error)) return
        setLoadError(errorMessage(error))
      }
      if (poll && !controller.signal.aborted) timer = setTimeout(() => { void load() }, 3000)
    }
    void load()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [token, project, refresh, authError])

  const job = view?.jobs.find(job => job.id === selected) ?? view?.jobs[0]
  const pending = view?.jobs.some(activeJob) ?? false
  const show = expanded ?? !job?.accepted_at
  async function submit() {
    const text = message.trim() || t.defaultMessage
    if (request.current?.message !== text) request.current = { request_id: crypto.randomUUID(), message: text }
    reads.current?.abort()
    setBusy('request'); setError('')
    try {
      const data = await requestOrganization(token, project, request.current)
      if (!mounted.current) return
      setView(data); setSelected(data.jobs[0]?.id ?? null); setExpanded(true); setComposing(false); setMessage(''); request.current = null
    } catch (error) {
      if (mounted.current && !authError(error)) setError(errorMessage(error))
    } finally { if (mounted.current) { setBusy(null); setRefresh(n => n + 1) } }
  }
  async function accept(id: string) {
    reads.current?.abort()
    setBusy('accept'); setError('')
    try {
      const data = await acceptOrganization(token, project, id)
      if (!mounted.current) return
      setView(data); onAccepted(); setExpanded(false)
    } catch (error) {
      if (mounted.current && !authError(error)) setError(errorMessage(error))
    } finally { if (mounted.current) { setBusy(null); setRefresh(n => n + 1) } }
  }
  return <section aria-label={t.organize} className="min-w-0 space-y-3">
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="outline" disabled={!canOrganize || pending || busy !== null} title={!canOrganize ? t.permission : undefined} onClick={() => { setComposing(true); setError('') }}><Sparkles size={15} aria-hidden />{t.organize}</Button>
      {job && <span role="status" className="text-muted-foreground flex min-w-0 items-center gap-1.5 text-xs">{activeJob(job) ? <LoaderCircle size={14} aria-hidden /> : job.accepted_at ? <CheckCircle2 size={14} aria-hidden /> : job.status === 'failed' ? <AlertTriangle size={14} aria-hidden /> : null}{job.accepted_at ? t.accepted : t[job.status]}</span>}
      {job && <Button variant="ghost" size="sm" onClick={() => setExpanded(!show)}>{show ? t.hide : t.show}</Button>}
      {!view && !loadError && <span role="status" className="text-muted-foreground text-xs">{t.loading}</span>}
    </div>
    {loadError && <div role="alert" className="text-destructive flex flex-wrap items-center gap-2 break-words text-sm"><span>{loadError}</span><Button variant="outline" size="sm" onClick={() => setRefresh(n => n + 1)}>{t.retry}</Button></div>}
    {error && <p role="alert" className="text-destructive break-words text-sm">{error}</p>}
    {composing && <form className="bg-card space-y-3 rounded-xl border p-4" onSubmit={event => { event.preventDefault(); void submit() }}>
      <p className="text-muted-foreground text-sm">{t.intro}</p><p className="text-muted-foreground text-xs">{t.savedSpecification}</p>
      <label className="block space-y-1 text-sm"><span>{t.instructions}</span><Textarea value={message} onChange={event => setMessage(event.target.value)} placeholder={t.placeholder} maxLength={8000} disabled={busy !== null} /></label>
      <div className="flex flex-wrap gap-2"><Button disabled={busy !== null || !canOrganize || pending}>{busy === 'request' ? t.requesting : t.request}</Button><Button type="button" variant="ghost" disabled={busy !== null} onClick={() => setComposing(false)}>{t.cancel}</Button></div>
    </form>}
    {job && show && <div className="bg-card min-w-0 space-y-4 rounded-xl border p-4">
      {view && view.jobs.length > 1 && <label className="block space-y-1 text-sm"><span>{t.history}</span><select className="border-input bg-background h-9 w-full min-w-0 rounded-lg border px-2 text-sm" value={job.id} disabled={busy !== null} onChange={event => { setSelected(event.target.value); setExpanded(true); setError('') }}>{view.jobs.map(item => <option key={item.id} value={item.id}>{new Date(item.created_at).toLocaleString(lang)} · {item.accepted_at ? t.accepted : t[item.status]}</option>)}</select></label>}
      <p className="text-muted-foreground text-xs">{t.savedSpecification}</p>
      <p className="text-muted-foreground text-xs">{t.timestamp}: {new Date(job.created_at).toLocaleString(lang)}</p>
      {activeJob(job) && <div className="space-y-1 text-sm"><p role="status" className="text-muted-foreground">{t[job.status]}</p>{job.status === 'queued' && <p className="text-muted-foreground">{t.queueHint} <a className="underline" href="/agent-queues">{t.queueLink}</a></p>}</div>}
      {job.status === 'failed' && <div role="alert" className="space-y-2 text-sm"><p className="text-destructive break-words">{job.error || t.failed}</p><p className="text-muted-foreground">{t.failedHint}</p></div>}
      {job.status === 'completed' && !job.proposal && <p role="alert" className="text-destructive text-sm">{t.noProposal}</p>}
      {job.proposal && <Proposal job={job} t={t} />}
      {job.accepted_at ? <p className="text-muted-foreground text-sm">{t.acceptedNote}</p> : job.status === 'completed' && job.proposal && <div className="space-y-2 border-t pt-3"><p className="text-muted-foreground text-sm">{t.acceptanceNote}</p><Button disabled={!canOrganize || busy !== null || pending || job.proposal.groups.length === 0} title={!canOrganize ? t.permission : undefined} onClick={() => void accept(job.id)}>{busy === 'accept' ? t.accepting : t.accept}</Button></div>}
      {view && view.total > view.jobs.length && <p className="text-muted-foreground text-xs">{t.limited.replace('{n}', String(view.jobs.length)).replace('{total}', String(view.total))}</p>}
      {view && view.messages.length > 0 && <details className="min-w-0 border-t pt-3"><summary className="cursor-pointer text-sm">{t.conversation}</summary><div className="mt-3 max-h-80 space-y-3 overflow-y-auto">{view.messages.map(entry => <div key={entry.id} className="min-w-0 break-words border-l-2 pl-3"><p className="text-muted-foreground mb-1 text-xs">{entry.role} · {new Date(entry.created_at).toLocaleString(lang)}</p><Markdown text={entry.body} /></div>)}</div></details>}
    </div>}
  </section>
}

function Proposal({ job, t }: { job: OrganizerJob; t: Labels }) {
  const proposal = job.proposal!
  const tickets = new Map(job.snapshot.tickets.map(ticket => [ticket.id, ticket]))
  return <div className="min-w-0 space-y-4">
    {proposal.groups.length === 0 && <p className="text-muted-foreground text-sm">{t.noChanges}</p>}
    {proposal.groups.map((group, index) => {
      const previous = job.snapshot.lanes.find(lane => lane.id === group.lane_id)
      return <article key={`${group.lane_id ?? 'new'}:${index}`} className="min-w-0 space-y-3 rounded-lg border p-3">
        <div className="flex flex-wrap items-center gap-2"><h3 className="min-w-0 break-words font-semibold">{group.title}</h3><span className="text-muted-foreground bg-muted rounded-full px-2 py-0.5 text-xs">{group.lane_id ? t.existingLane : t.newLane}</span><span className={group.readiness === 'ready' ? 'text-muted-foreground text-xs' : 'text-amber-600 text-xs'}>{t[group.readiness]}</span></div>
        <p className="break-words text-sm">{group.purpose}</p>
        <div className="space-y-1"><h4 className="text-muted-foreground text-xs font-medium">{t.reason}</h4><p className="whitespace-pre-wrap break-words text-sm">{group.reason}</p></div>
        <div className="space-y-1"><h4 className="text-muted-foreground text-xs font-medium">{t.tickets}</h4><ul className="space-y-1">{group.ticket_ids.map(id => <li key={id} className="break-words text-sm"><a href={`/board#t=${encodeURIComponent(id)}`} className="underline">{tickets.get(id)?.title || id}</a><span className="text-muted-foreground ml-2 text-xs">{id}</span></li>)}</ul></div>
        <div className="min-w-0 space-y-1 break-words"><h4 className="text-muted-foreground text-xs font-medium">{t.context}</h4><Markdown text={group.context} /></div>
        {previous && <details className="min-w-0 text-sm"><summary className="cursor-pointer">{t.currentContext}</summary><div className="mt-2 break-words"><Markdown text={previous.context} /></div></details>}
        <details className="min-w-0 text-sm"><summary className="cursor-pointer">{t.source}</summary><div className="mt-2 space-y-3">{group.ticket_ids.map(id => { const ticket = tickets.get(id); return <div key={id} className="min-w-0 space-y-1 border-t pt-2"><h5 className="break-words font-medium">{ticket?.title || id}</h5><p className="text-muted-foreground text-xs">{ticket?.state}</p><Markdown text={ticket?.body} />{ticket?.links && <dl className="space-y-1 text-xs">{Object.entries(ticket.links).map(([key, value]) => <div key={key} className="break-all"><dt className="font-medium">{key}</dt><dd>{value}</dd></div>)}</dl>}</div> })}</div></details>
      </article>
    })}
    {proposal.groups.length > 0 && <p className="text-muted-foreground text-xs">{t.readinessNote}</p>}
    {proposal.unassigned.length > 0 && <div className="space-y-2"><h3 className="text-sm font-semibold">{t.unassigned}</h3><ul className="space-y-2">{proposal.unassigned.map(item => <li key={item.ticket_id} className="break-words text-sm"><a className="underline" href={`/board#t=${encodeURIComponent(item.ticket_id)}`}>{tickets.get(item.ticket_id)?.title || item.ticket_id}</a><p className="text-muted-foreground">{item.reason}</p></li>)}</ul></div>}
  </div>
}
