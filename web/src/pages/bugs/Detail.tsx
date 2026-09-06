import { useCallback, useEffect, useRef, useState } from 'react'
import { api } from '@/lib/api'
import { bugPath, json, type Bug, type BugJob, type ResearchConfig } from '@/lib/bugs'
import { pick, type Locale } from '@/lib/i18n'
import { isAuthError } from '@/lib/session'
import { Button } from '@/components/ui/button'
import { Markdown } from '@/components/Markdown'
import { fieldClass } from './Report'
import { STR } from './strings'
export function Detail({ bug, token, lang, canWrite, canReview, canConfigure, refresh, onAuthError }: { bug: Bug; token: string; lang: Locale; canWrite: boolean; canReview: boolean; canConfigure: boolean; refresh: () => void; onAuthError: () => void }) {
  const t = pick(STR, lang)
  const label = (key: string) => t[key as keyof typeof t] ?? key
  const [config, setConfig] = useState<ResearchConfig | null>(null)
  const [jobs, setJobs] = useState<BugJob[]>([])
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [guidance, setGuidance] = useState('')
  const [note, setNote] = useState(bug.note ?? '')
  const [triage, setTriage] = useState(bug.triage)
  const [severity, setSeverity] = useState(bug.severity)
  const [reviewDirty, setReviewDirty] = useState(false)
  const [duplicate, setDuplicate] = useState(bug.duplicate_of ?? '')
  const [configDraft, setConfigDraft] = useState<ResearchConfig | null>(null)
  useEffect(() => { if (!reviewDirty) { setTriage(bug.triage); setSeverity(bug.severity); setDuplicate(bug.duplicate_of ?? ''); setNote(bug.note ?? '') } }, [bug.triage, bug.severity, bug.duplicate_of, bug.note, reviewDirty])
  const requestIds = useRef(new Map<string, string>())
  const configPath = `/projects/${encodeURIComponent(bug.ticket.project)}/bug-research-config`
  const fail = useCallback((e: unknown) => { if (isAuthError(e)) onAuthError(); else setError(e instanceof Error ? e.message : String(e)) }, [onAuthError])
  const load = useCallback(async () => {
    const [settings, history] = await Promise.all([api<ResearchConfig>(token, configPath), api<{jobs: BugJob[]}>(token, `${bugPath(bug.ticket.id)}/research`)])
    setConfig(settings); setJobs(history.jobs)
  }, [token, configPath, bug.ticket.id])
  useEffect(() => { let active = true; const run = () => { if (active) void load().catch(e => { if (active) fail(e) }) }; run(); const timer = setInterval(run, 5000); return () => { active = false; clearInterval(timer) } }, [load, fail])
  const mutate = async (path: string, body: unknown, method = 'POST', requestKey?: string) => {
    if (busy) return
    setBusy(true); setError('')
    try {
      let payload = body
      if (requestKey) {
        if (!requestIds.current.has(requestKey)) requestIds.current.set(requestKey, crypto.randomUUID())
        payload = { ...(body as object), request_id: requestIds.current.get(requestKey) }
      }
      await api(token, path, { method, ...json(payload) })
      if (method === 'PATCH') setReviewDirty(false)
      if (requestKey) requestIds.current.delete(requestKey)
      await load(); refresh()
    } catch (e) { fail(e) } finally { setBusy(false) }
  }
  const activeJob = jobs.find(job => job.status === 'queued' || job.status === 'running')
  return <section className="min-w-0 space-y-4 rounded-xl border bg-card p-4">
    <div><a className="text-sm text-primary underline" href={`/board#t=${encodeURIComponent(bug.ticket.id)}`}>{t.ticket}: {bug.ticket.id}</a><h2 className="mt-1 break-words text-lg font-semibold">{bug.ticket.title}</h2><p className="text-xs text-muted-foreground">{t.status}: {bug.ticket.state}</p></div>
    <Markdown text={bug.ticket.body} />
    <form className="space-y-3 border-t pt-3" onSubmit={e => { e.preventDefault(); void mutate(bugPath(bug.ticket.id), { triage, severity, duplicate_of: duplicate || null, note: note || undefined }, 'PATCH') }}>
      <h3 className="font-medium">{t.review}</h3>{bug.updated_by && <p className="text-xs text-muted-foreground">{label(bug.triage)} · {bug.updated_by}{bug.updated_at ? ` · ${new Date(bug.updated_at).toLocaleString(lang)}` : ''}</p>}
      <div className="grid min-w-0 gap-2 md:grid-cols-2"><label className="text-sm">{t.triage}<select className={fieldClass} value={triage} disabled={!canReview || busy} onChange={e => { setReviewDirty(true); setTriage(e.target.value) }}>{['needs_triage', 'ready_for_review', 'confirmed', 'needs_information', 'duplicate', 'not_a_bug'].map(key => <option key={key} value={key}>{label(key)}</option>)}</select></label><label className="text-sm">{t.severity}<select className={fieldClass} value={severity} disabled={!canReview || busy} onChange={e => { setReviewDirty(true); setSeverity(e.target.value) }}>{['unknown', 'critical', 'high', 'medium', 'low'].map(key => <option key={key} value={key}>{label(key)}</option>)}</select></label></div>
      {triage === 'duplicate' && <label className="block text-sm">{t.duplicateOf}<input required className={fieldClass} value={duplicate} disabled={!canReview || busy} onChange={e => { setReviewDirty(true); setDuplicate(e.target.value) }} /></label>}
      {canReview && <><label className="block text-sm">{t.note}<textarea className={fieldClass} value={note} disabled={busy} onChange={e => { setReviewDirty(true); setNote(e.target.value) }} /></label><Button type="submit" disabled={busy}>{t.save}</Button></>}
    </form>
    <div className="space-y-3 border-t pt-3"><h3 className="font-medium">{t.research}</h3><p className="text-xs text-muted-foreground">{t.reviewHint}</p>
      {config?.enabled ? <p className="break-words text-sm">{t.repository}: {config.repository} · {t.revision}: {config.revision}</p> : <p className="text-sm text-muted-foreground">{t.notConfigured}</p>}
      {canConfigure && <details><summary className="cursor-pointer text-sm" onClick={() => setConfigDraft(config ?? {repository: '', revision: 'HEAD', enabled: false})}>{t.config}</summary>{configDraft && <form className="mt-2 space-y-2" onSubmit={e => { e.preventDefault(); void mutate(configPath, configDraft, 'PUT') }}><label className="block text-sm">{t.repository}<input className={fieldClass} required value={configDraft.repository} onChange={e => setConfigDraft({...configDraft, repository: e.target.value})} /></label><label className="block text-sm">{t.revision}<input className={fieldClass} required value={configDraft.revision} onChange={e => setConfigDraft({...configDraft, revision: e.target.value})} /></label><label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={configDraft.enabled} onChange={e => setConfigDraft({...configDraft, enabled: e.target.checked})} />{t.enabled}</label><Button disabled={busy}>{t.save}</Button></form>}</details>}
      {canWrite && <><label className="block text-sm">{t.guidance}<textarea className={fieldClass} value={guidance} onChange={e => setGuidance(e.target.value)} disabled={busy} /></label><div className="flex flex-wrap gap-2">{activeJob ? <><Button disabled={busy || !guidance.trim()} onClick={() => void mutate(`/agent-jobs/${encodeURIComponent(activeJob.id)}/steer`, { message: guidance }, 'POST', `steer:${activeJob.id}:${guidance}`)}>{t.steer}</Button><Button variant="outline" disabled={busy} onClick={() => void mutate(`/agent-jobs/${encodeURIComponent(activeJob.id)}/cancel`, {})}>{t.cancel}</Button></> : <Button disabled={busy || !config?.enabled || !config.repository} onClick={() => void mutate(`${bugPath(bug.ticket.id)}/research`, { message: guidance || undefined }, 'POST', `research:${guidance}`)}>{jobs.length ? t.retry : t.research}</Button>}</div></>}
    </div>
    {error && <p role="alert" className="break-words text-sm text-destructive">{error}</p>}
    <section className="space-y-2 border-t pt-3"><h3 className="font-medium">{t.history}</h3>{!jobs.length && <p className="text-sm text-muted-foreground">{t.noRuns}</p>}{jobs.map(job => <details key={job.id} className="min-w-0 rounded-lg border p-3"><summary className="cursor-pointer break-words text-sm">{label(job.status)} · {new Date(job.created_at).toLocaleString(lang)} · {job.id}</summary>{job.repository_revision && <p className="my-2 break-all text-xs">{t.revision}: {job.repository_revision}</p>}{(job.prompt || job.snapshot) && <details className="mt-3 min-w-0"><summary className="cursor-pointer text-sm font-medium">{t.researchInput}</summary>{job.prompt && <><h4 className="mt-2 text-xs font-medium">{t.originalRequest}</h4><pre className="whitespace-pre-wrap break-words text-xs [overflow-wrap:anywhere]">{job.prompt}</pre></>}{job.snapshot && <><h4 className="mt-2 text-xs font-medium">{t.reportSnapshot}</h4><pre className="whitespace-pre-wrap break-words text-xs [overflow-wrap:anywhere]">{job.snapshot}</pre></>}</details>}{job.evidence && <div className="mt-2 space-y-1 text-xs"><h4 className="font-medium">{t.evidence}</h4>{job.evidence.runtime_reproduced === false && <p className="text-muted-foreground">{t.inspectionOnly}</p>}{job.evidence.inspected?.map((entry, index) => <p key={index} className="break-all font-mono">{entry.path}:{entry.start_line}–{entry.end_line} · {entry.revision}</p>)}</div>}{job.steering?.map(entry => <p key={entry.id} className="mt-2 whitespace-pre-wrap break-words text-sm">{t.guidance}: {entry.message}</p>)}{job.error && <p className="text-sm text-destructive">{job.error}</p>}{job.response && <div className="mt-3 min-w-0"><h4 className="text-sm font-medium">{t.result}</h4><Markdown text={job.response} /></div>}</details>)}</section>
  </section>
}
