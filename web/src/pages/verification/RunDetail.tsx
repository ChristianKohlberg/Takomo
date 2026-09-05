import { useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Markdown } from '@/components/Markdown'
import { runRequest, type RunCase, type TestRun } from '@/lib/test-runs'
import { useSpecification } from '../specification/context'

export function RunDetail({ run, changed }: { run: TestRun; changed: (run: TestRun) => void }) {
  const { token, actor, scopes, onError, lang } = useSpecification()
  const de = lang === 'de'
  const [busy, setBusy] = useState(false)
  const [retryKey] = useState(() => crypto.randomUUID())
  const writable = scopes.includes('write') && run.kind === 'execution'
  const mutate = (action: string) => {
    setBusy(true)
    void runRequest<TestRun>(token, `/test-runs/${run.id}${action === 'retry' ? '/retry' : ''}`, action === 'retry' ? { idempotency_key: retryKey } : { action }, action === 'retry' ? 'POST' : 'PATCH').then(changed).catch(onError).finally(() => setBusy(false))
  }
  return <article className="min-w-0 rounded-xl border bg-card p-4 md:p-5">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0"><h2 className="font-semibold">{de ? 'Testlauf' : 'Test run'} · {run.status}</h2><p className="break-all text-xs text-muted-foreground">{run.id}</p></div>
      {writable && <div className="flex flex-wrap gap-2">
        {run.status === 'queued' && <Button disabled={busy} onClick={() => mutate('start')}>{de ? 'Ausführung übernehmen' : 'Claim execution'}</Button>}
        {run.status === 'running' && run.executor === actor && <Button disabled={busy} onClick={() => mutate('complete')}>{de ? 'Abschließen' : 'Complete run'}</Button>}
        {((run.status === 'queued' && run.created_by === actor) || (run.status === 'running' && run.executor === actor)) && <Button variant="outline" disabled={busy} onClick={() => mutate('cancel')}>{de ? 'Abbrechen' : 'Cancel run'}</Button>}
        {['completed', 'cancelled'].includes(run.status) && <Button variant="outline" disabled={busy} onClick={() => mutate('retry')}>{de ? 'Gleichen Stand erneut testen' : 'Retry same revisions'}</Button>}
      </div>}
    </div>
    <dl className="my-4 grid min-w-0 grid-cols-1 gap-3 text-sm md:grid-cols-3">
      <div><dt className="text-xs text-muted-foreground">{de ? 'Codeversion' : 'Code version'}</dt><dd className="break-all">{run.code_ref ?? '—'}</dd></div>
      <div><dt className="text-xs text-muted-foreground">{de ? 'Umgebung' : 'Environment'}</dt><dd className="break-words">{run.environment_snapshot?.name ?? run.environment ?? '—'}</dd></div>
      <div><dt className="text-xs text-muted-foreground">{de ? 'Ausführende Person / Agent' : 'Executor'}</dt><dd className="break-all">{run.executor ?? '—'}</dd></div>
    </dl>
    {run.kind === 'legacy' && <p className="mb-4 rounded-md bg-muted p-3 text-sm">{de ? 'Historischer Nachweis. Definition und Spezifikationsstand sind unbekannt.' : 'Legacy evidence. The original definition and specification revisions are unknown.'}</p>}
    {run.retry_of && <p className="mb-3 break-all text-xs text-muted-foreground">{de ? 'Wiederholung von' : 'Retry of'} {run.retry_of}</p>}
    <div className="grid gap-3">{run.cases.map(c => <CaseResult key={c.case} item={c} run={run} changed={changed} />)}</div>
  </article>
}
function CaseResult({ item, run, changed }: { item: RunCase; run: TestRun; changed: (run: TestRun) => void }) {
  const { token, actor, scopes, onError, lang } = useSpecification()
  const de = lang === 'de'
  const [note, setNote] = useState('')
  const [evidence, setEvidence] = useState('')
  const [busy, setBusy] = useState(false)
  const attempt = useRef<{ fingerprint: string; key: string } | null>(null)
  const captured = run.definitions[item.check]
  const definition = captured?.definition
  const specification = captured?.specification
  const policy = definition?.verification
  const agentPassed = item.results.some(r => r.actor_kind === 'agent' && r.verdict === 'pass')
  const kind = policy === 'human' || (policy === 'agent_then_human' && agentPassed) ? 'human' : 'agent'
  const canRecord = scopes.includes('write') && run.kind === 'execution' &&
    !item.results.some(r => r.actor_kind === kind) &&
    (kind === 'human' ? scopes.includes('human') && ['running', 'completed'].includes(run.status) : run.status === 'running' && run.executor === actor)
  return <section className="min-w-0 rounded-lg border p-3">
    <h3 className="break-words font-medium">{definition?.title ?? item.check} / {item.snapshot?.label || item.snapshot?.key || item.case}</h3>
    {definition && <details className="my-2 text-sm"><summary className="cursor-pointer text-muted-foreground">{de ? 'Festgehaltene Definition' : 'Captured definition'}</summary>
      {definition.precondition && <p className="mt-2 whitespace-pre-wrap">{definition.precondition}</p>}
      <Markdown text={definition.body} />
      <pre className="max-w-full overflow-x-auto text-xs">{JSON.stringify(item.snapshot?.assignment, null, 2)}</pre>
      <p className="break-all text-xs text-muted-foreground">{item.definition_revision}<br />{item.specification_revision}</p>
    </details>}
    {specification && <details className="my-2 text-sm"><summary className="cursor-pointer text-muted-foreground">{de ? 'Festgehaltener Spezifikationsstand' : 'Captured specification'}</summary>
      {specification.sections.map(section => <div key={section.id} className="mt-2"><h4 className="break-words font-medium">{section.title}</h4><p className="whitespace-pre-wrap break-words">{section.notes}</p></div>)}
    </details>}
    {item.results.map(result => <div key={result.id} className="mt-2 rounded-md bg-muted px-3 py-2 text-sm">
      <p className="font-medium">{result.actor_kind === 'human' ? (de ? 'Menschliche Prüfung' : 'Human review') : (de ? 'Ausführung' : 'Execution')} · {result.verdict}</p>
      <p className="break-all text-xs text-muted-foreground">{result.actor} · {new Date(result.recorded_at).toLocaleString()}</p>
      {result.note && <p className="mt-1 whitespace-pre-wrap break-words">{result.note}</p>}
      {result.evidence.map((e, i) => <p key={i} className="break-all">{/^https?:\/\//i.test(e) ? <a className="text-primary underline" href={e} target="_blank" rel="noreferrer">{e}</a> : e}</p>)}
    </div>)}
    {canRecord && <div className="mt-3 grid gap-2">
      <label className="grid gap-1 text-xs">{de ? 'Beobachtung (bei Fehler erforderlich)' : 'Observation (required for a failure)'}<Input value={note} onChange={e => setNote(e.target.value)} /></label>
      <label className="grid gap-1 text-xs">{de ? 'Nachweis (URL oder Referenz)' : 'Evidence (URL or reference)'}<Input value={evidence} onChange={e => setEvidence(e.target.value)} maxLength={2048} /></label>
      <div className="flex flex-wrap gap-2">{(['pass', 'fail', 'blocked', 'unreachable'] as const).map(verdict => <Button key={verdict} size="sm" variant={verdict === 'pass' ? 'default' : 'outline'} disabled={busy || (verdict !== 'pass' && !note.trim())} onClick={() => {
        setBusy(true)
        const request = { case: item.case, actor_kind: kind, verdict, note: note || null, evidence: evidence ? [evidence] : [] }
        const fingerprint = JSON.stringify(request)
        if (attempt.current?.fingerprint !== fingerprint) attempt.current = { fingerprint, key: crypto.randomUUID() }
        void runRequest<TestRun>(token, `/test-runs/${run.id}/results`, { ...request, idempotency_key: attempt.current.key }).then(changed).catch(onError).finally(() => setBusy(false))
      }}>{kind === 'human' && verdict === 'pass' ? (de ? 'Bestätigen' : 'Approve') : verdict}</Button>)}</div>
    </div>}
  </section>
}
