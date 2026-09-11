import { useLiveRefresh } from '@/hooks/useLiveRefresh'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Markdown } from '@/components/Markdown'
import { getAgentJob, JOB_STATUSES, listAgentJobs, type AgentJob, type AgentJobDetail, type AgentJobList, type AgentJobStatus } from '@/lib/agent-queues'
import { specificationLink } from '@/lib/specification-url'
import { isAuthError } from '@/lib/session'
import { pick, type Locale } from '@/lib/i18n'
import { STR } from './strings'

type Labels = typeof STR.en
const date = (value: number | null, lang: Locale, missing: string) => value == null ? missing : new Date(value).toLocaleString(lang)
const errorText = (error: unknown, fallback: string) => error instanceof Error ? error.message : fallback

const jobTitle = (job: AgentJob, t: Labels) => job.kind === 'codebase_import' ? `${t.extraction} · ${job.source?.full_name ?? job.project}` : job.kind === 'ticket_document_classify' ? `${t.documentClassification} · ${job.ticket_id ?? job.node}` : (job.kind === 'document_chat' || job.kind === 'document_workspace') ? t.documentDiscussion : job.kind === 'lane_organize' ? t.organizeLanes : job.kind === 'bug_research' ? job.ticket_id ?? job.node : job.section_title || job.node

function Status({ status, t }: { status: AgentJobStatus; t: Labels }) {
  return <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${status === 'failed' ? 'bg-destructive/10 text-destructive' : status === 'running' ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground'}`}>{t[status]}</span>
}

function Details({ token, project, id, automatic, refresh, lang, onAuthError }: {
  token: string; project: string; id: string; automatic: boolean; refresh: number; lang: Locale; onAuthError: () => void
}) {
  const t = pick(STR, lang)
  const sectionRef = useRef<HTMLElement>(null)
  useEffect(() => {
    if (window.matchMedia?.('(max-width: 767px)').matches) {
      sectionRef.current?.scrollIntoView({ block: 'start' })
      sectionRef.current?.focus({ preventScroll: true })
    }
  }, [])
  const [data, setData] = useState<AgentJobDetail | null>(null)
  const [error, setError] = useState('')
  useLiveRefresh({
    token, project, scope: `${id}:${refresh}`, topics: ['agent'], automatic,
    activeMs: data && ['queued', 'running'].includes(data.job.status) ? 3000 : false,
    onError: e => { if (isAuthError(e)) onAuthError(); else setError(errorText(e, t.detailUnavailable)) },
    load: async signal => {
      const result = await getAgentJob(token, id, signal)
      if (!signal.aborted) { setData(result); setError('') }
    },
  })
  const j = data?.job
  const fields: [string, string | number | null][] = j ? [
    [t.job, j.id], [t.status, t[j.status]], [t.requestedBy, j.requested_by],
    [t.started, date(j.started_at ?? null, lang, t.noValue)], [t.created, date(j.created_at, lang, t.noValue)], [t.finished, date(j.finished_at, lang, t.noValue)],
    [t.lease, date(j.lease_expires_at, lang, t.noValue)], [t.deadline, date(j.deadline, lang, t.noValue)],
    [t.model, j.telemetry?.model ?? null], [t.worker, j.service_id], [t.boundWorker, j.conversation_service_id],
    [t.conversation, j.conversation_id], [t.attempt, j.attempt_id], [t.thread, j.telemetry?.thread_id ?? j.thread_id], [t.turn, j.telemetry?.turn_id ?? j.turn_id],
    ...(j.kind === 'codebase_import' ? [[t.repository, j.source?.full_name ?? t.noValue], [t.scope, j.source?.scope.include.join(', ') ?? t.noValue], [t.revision, j.source_revision]] as [string, string][] : j.kind === 'lane_organize' ? [[t.revision, j.source_revision]] as [string, string][] : j.kind === 'ticket_document_classify' ? [[t.openTicket, j.ticket_id ?? j.node], [t.revision, j.source_revision]] as [string, string][] : j.kind === 'bug_research' ? [[t.openBug, j.ticket_id ?? j.node], [t.revision, j.repository_revision ?? t.noValue]] as [string, string][] : [[t.map, j.mindmap], ...((j.kind === 'document_chat' || j.kind === 'document_workspace') ? [] : [[t.node, j.node]]), [t.revision, j.source_revision]] as [string, string | null][]),
  ] : []
  return <section ref={sectionRef} tabIndex={-1} aria-label={t.detail} className="bg-card border-border order-first min-w-0 rounded-xl border p-4 md:order-last">
    <h2 className="text-base font-semibold">{t.detail}</h2>
    {error && <div role="alert" className="text-destructive mt-3 break-words text-sm">{data && <p>{t.stale}</p>}<p>{error}</p></div>}
    {!j ? !error && <p role="status" className="text-muted-foreground mt-3 text-sm">{t.loading}</p> : <>
      <h3 className="mt-3 break-words font-medium">{jobTitle(j, t)}</h3>
      <p className="text-muted-foreground mt-1 break-words text-xs">{j.project}</p>
      <a href={j.kind === 'lane_organize' ? `/lanes?project=${encodeURIComponent(j.project)}` : (j.kind === 'bug_research' || j.kind === 'ticket_document_classify') ? `/board#t=${encodeURIComponent(j.ticket_id ?? j.node)}` : specificationLink(j.project, 'document', (j.kind === 'codebase_import' || j.kind === 'document_chat' || j.kind === 'document_workspace') ? undefined : j.node)} className="text-primary mt-2 inline-block text-sm underline">{j.kind === 'ticket_document_classify' ? t.openTicket : (j.kind === 'codebase_import' || j.kind === 'document_chat' || j.kind === 'document_workspace') ? t.openDocument : j.kind === 'lane_organize' ? t.openLanes : j.kind === 'bug_research' ? t.openBug : t.openSection}</a>
      {j.kind === 'codebase_import' && <a className="text-primary ml-3 text-sm underline" href={specificationLink(j.project, 'map')}>{t.openMindmap}</a>}
      {j.status === 'running' && j.telemetry?.phase && <p className="mt-3 text-sm">{({ preparing_source: t.preparing, drafting: t.drafting, publishing: t.publishing })[j.telemetry.phase]}</p>}
      <Usage job={j} lang={lang} />
      {j.status === 'queued' && <p className="text-muted-foreground mt-3 text-sm">{j.conversation_service_id ? t.boundWaiting : t.waiting}</p>}
      {j.error && <div className="text-destructive mt-4"><h3 className="text-sm font-semibold">{t.error}</h3><pre className="mt-1 whitespace-pre-wrap break-all text-xs">{j.error}</pre></div>}
      <dl className="mt-4 grid min-w-0 grid-cols-1 gap-x-4 gap-y-1 text-xs md:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
        {fields.map(([label, value]) => <Field key={label} label={label} value={value ?? t.noValue} />)}
      </dl>
      <p className="text-muted-foreground mt-2 text-xs">{t.timestampHint}</p>
      {j.kind === 'codebase_import' && j.status === 'completed' && <p className="mt-4 text-sm">{t.extractionReady}</p>}
      {j.kind !== 'codebase_import' && <><h3 className="mt-5 text-sm font-semibold">{t.prompt}</h3>
      <pre className="bg-muted mt-2 rounded-lg p-3 text-sm whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{j.prompt}</pre>
      <h3 className="mt-5 text-sm font-semibold">{t.response}</h3>
      {j.response ? <Markdown diagramAccess={{ token, project: j.project }} text={j.response} className="mt-2 min-w-0 text-sm [overflow-wrap:anywhere]" /> : <p className="text-muted-foreground mt-2 text-sm">{t.noResponse}</p>}
      </>}
      <details className="mt-5"><summary className="cursor-pointer text-sm font-semibold">{j.kind === 'codebase_import' ? t.sourceSnapshot : j.kind === 'lane_organize' ? t.laneSnapshot : j.kind === 'bug_research' ? t.bugSnapshot : t.snapshot}</summary><pre className="bg-muted mt-2 rounded-lg p-3 text-xs whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{j.snapshot}</pre></details>
      {j.kind !== 'codebase_import' && <details className="mt-5"><summary className="cursor-pointer text-sm font-semibold">{t.history} ({data.messages.length})</summary>
        <div className="mt-3 space-y-3">{data.messages.map(message => <div key={message.id} className="border-border rounded-lg border p-3">
          <p className="text-muted-foreground text-xs">{message.role === 'assistant' ? t.codex : t.member} · {date(message.created_at, lang, t.noValue)}</p>
          <Markdown diagramAccess={{ token, project: j.project }} text={message.body} className="mt-2 text-sm [overflow-wrap:anywhere]" />
          <p className="text-muted-foreground mt-2 break-all text-xs">{message.job_id}</p>
        </div>)}</div>
      </details>}
    </>}
  </section>
}

function Usage({ job, lang }: { job: AgentJob; lang: Locale }) {
  const t = pick(STR, lang)
  const usage = job.telemetry?.usage
  return <section className="border-border mt-4 rounded-lg border p-3" aria-label={t.tokens}>
    <h3 className="text-sm font-semibold">{t.tokens}</h3>
    {usage ? <dl className="mt-2 grid grid-cols-2 gap-2 text-xs">{([
      [t.inputTokens, usage.input_tokens], [t.cachedTokens, usage.cached_input_tokens],
      [t.outputTokens, usage.output_tokens], [t.reasoningTokens, usage.reasoning_output_tokens], [t.totalTokens, usage.total_tokens],
    ] as [string, number][]).map(([label, count]) => <div key={label}><dt className="text-muted-foreground">{label}</dt><dd className="font-medium tabular-nums">{count.toLocaleString(lang)}</dd></div>)}</dl> : <p className="text-muted-foreground mt-2 text-xs">{t.noUsage}</p>}
    <p className="text-muted-foreground mt-2 text-xs">{t.usageHint}</p>
  </section>
}

function Field({ label, value }: { label: string; value: string | number }) {
  return <><dt className="text-muted-foreground pt-1">{label}</dt><dd className="break-all pb-1 font-mono md:pt-1">{value}</dd></>
}

/** The caller keys this component by token and project to drop old data immediately on scope changes. */
export function Inspector({ token, project, lang, onAuthError }: { token: string; project: string; lang: Locale; onAuthError: () => void }) {
  const t = pick(STR, lang)
  const [status, setStatus] = useState<AgentJobStatus | ''>('')
  const [automatic, setAutomatic] = useState(true)
  const [refresh, setRefresh] = useState(0)
  const [data, setData] = useState<AgentJobList | null>(null)
  const [selected, setSelected] = useState(() => new URLSearchParams(window.location.search).get('run') ?? '')
  const [error, setError] = useState('')
  const [updated, setUpdated] = useState<number | null>(null)
  useLiveRefresh({
    token, project, scope: `${status}:${refresh}`, topics: ['agent'], automatic,
    activeMs: data?.items.some(job => ['queued', 'running'].includes(job.status)) ? 3000 : false,
    onError: e => { if (isAuthError(e)) onAuthError(); else setError(errorText(e, t.unavailable)) },
    load: async signal => {
      const result = await listAgentJobs(token, project, status, signal)
      if (!signal.aborted) { setData(result); setError(''); setUpdated(Date.now()) }
    },
  })
  return <div className="mx-auto flex w-full max-w-7xl flex-col gap-4">
    <p className="text-muted-foreground text-sm">{t.description}</p>
    <div className="flex flex-wrap items-center gap-3">
      <label className="flex items-center gap-2 text-sm">{t.status}
        <select aria-label={t.status} value={status} onChange={e => { setStatus(e.target.value as AgentJobStatus | ''); setData(null); setError(''); setSelected(''); setUpdated(null) }} className="border-input bg-background rounded-md border px-3 py-2">
          <option value="">{t.all}</option>{JOB_STATUSES.map(s => <option key={s} value={s}>{t[s]}</option>)}
        </select>
      </label>
      <Button variant="outline" onClick={() => setRefresh(n => n + 1)}>{t.refresh}</Button>
      <label className="text-muted-foreground flex items-center gap-2 text-sm"><Checkbox checked={automatic} onCheckedChange={v => setAutomatic(v === true)} />{t.automatic}</label>
    </div>
    {data && <div className="grid grid-cols-2 gap-2 md:grid-cols-5">{JOB_STATUSES.map(s => <div key={s} className="bg-card border-border rounded-lg border px-3 py-2"><span className="text-muted-foreground text-xs">{t[s]}</span><p className="text-xl font-semibold tabular-nums">{data.counts[s]}</p></div>)}</div>}
    {error && <div role="alert" className="text-destructive break-words text-sm">{data && <p>{t.stale}</p>}<p>{error}</p></div>}
    {updated && <p className="text-muted-foreground text-xs">{t.lastUpdated}: {date(updated, lang, '')}</p>}
    <div className="grid min-w-0 items-start gap-4 md:grid-cols-2">
      <section aria-label={t.recent} className="min-w-0">
        <h2 className="mb-3 text-base font-semibold">{t.recent}</h2>
        {!data ? !error && <p role="status" className="text-muted-foreground text-sm">{t.loading}</p> : <>
          <p className="text-muted-foreground mb-3 text-xs">{t.count} {data.items.length} {t.of} {data.total}</p>
          {data.total > data.items.length && <p className="text-muted-foreground mb-3 text-sm">{t.limited}</p>}
          {data.items.length === 0 ? <p className="text-muted-foreground py-8 text-sm">{t.empty}</p> : <ul className="space-y-2">{data.items.map(j => <li key={j.id}>
            <button type="button" aria-pressed={selected === j.id} onClick={() => setSelected(j.id)} className={`bg-card w-full cursor-pointer rounded-lg border p-3 text-left ${selected === j.id ? 'border-primary ring-primary/20 ring-2' : 'border-border hover:bg-muted'}`}>
              <span className="flex flex-wrap items-start justify-between gap-2"><span className="min-w-0 flex-1 break-words text-sm font-medium [overflow-wrap:anywhere]">{jobTitle(j, t)}</span><Status status={j.status} t={t} /></span>
              <span className="text-muted-foreground mt-2 block break-all font-mono text-xs">{j.id}</span>
              <span className="text-muted-foreground mt-1 block break-words text-xs">{j.project} · {date(j.created_at, lang, '')}</span>
              <span className="text-muted-foreground mt-1 block text-xs">{t.tokens}: {j.telemetry?.usage ? j.telemetry.usage.total_tokens.toLocaleString(lang) : t.noValue}</span>
              {j.error && <span className="text-destructive mt-2 block line-clamp-2 break-all text-xs">{j.error}</span>}
            </button>
          </li>)}</ul>}
        </>}
      </section>
      {selected ? <Details project={project} key={selected} token={token} id={selected} automatic={automatic} refresh={refresh} lang={lang} onAuthError={onAuthError} /> : <p className="text-muted-foreground border-border rounded-xl border border-dashed p-5 text-sm">{t.select}</p>}
    </div>
    <p className="text-muted-foreground text-xs">{t.readOnly}</p>
  </div>
}
