import { useCallback, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router'
import { api } from '@/lib/api'
import { bugPath, listBugs, type Bug, type BugPage } from '@/lib/bugs'
import { BUG_SEVERITIES, BUG_VIEWS, PAGE_SIZE, RESEARCH_STATUSES, readBugsView, writeBugsView, type BugsView } from '@/lib/bugs-url'
import { pick, type Locale } from '@/lib/i18n'
import { isAuthError } from '@/lib/session'
import { Button } from '@/components/ui/button'
import { Report, fieldClass } from './Report'
import { Detail } from './Detail'
import { STR } from './strings'
export function Queue({ token, project, lang, onAuthError, canWrite, canReview, canConfigure }: {token: string; project: string; lang: Locale; onAuthError: () => void; canWrite: boolean; canReview: boolean; canConfigure: boolean}) {
  const t = pick(STR, lang)
  const label = (key: string) => t[key as keyof typeof t] ?? key
  const [params, setParams] = useSearchParams()
  const { view, severity, search, state, assignee, researchStatus, offset, bug: selected } = readBugsView(params)
  const update = useCallback((patch: Partial<BugsView>, replace = false) => setParams(prev => writeBugsView({ ...readBugsView(prev), ...patch }), { replace }), [setParams])
  const filter = (patch: Partial<BugsView>, replace = false) => update({ ...patch, offset: 0 }, replace)
  const [page, setPage] = useState<BugPage | null>(null)
  const [bug, setBug] = useState<Bug | null>(null)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [report, setReport] = useState(false)
  const [version, setVersion] = useState(0)
  const refresh = useCallback(() => setVersion(v => v + 1), [])
  useEffect(() => {
    if (!project) return
    const controller = new AbortController()
    let active = true
    const message = (e: unknown) => e instanceof Error ? e.message : String(e)
    const load = async () => {
      const [list, detail] = await Promise.allSettled([listBugs(token, project, view, severity, search, offset, controller.signal, {state, assignee, research_status: researchStatus}), selected ? api<Bug>(token, bugPath(selected), { signal: controller.signal }) : Promise.resolve(null)])
      if (!active) return
      if (list.status === 'fulfilled') { setPage(list.value); setError('') } else if (isAuthError(list.reason)) onAuthError(); else setError(message(list.reason))
      if (detail.status === 'fulfilled') {
        if (detail.value && detail.value.ticket.project !== project) { setBug(null); setNotice(t.linkedElsewhere); update({ bug: '' }, true) } else setBug(detail.value)
      } else if (isAuthError(detail.reason)) onAuthError(); else { setBug(null); setNotice(message(detail.reason)); update({ bug: '' }, true) }
    }
    void load()
    const timer = setInterval(() => void load(), 5000)
    return () => { active = false; controller.abort(); clearInterval(timer) }
  }, [token, project, view, severity, search, state, assignee, researchStatus, offset, selected, version, onAuthError, update, t.linkedElsewhere])
  if (!project) return <p className="text-muted-foreground">{t.projectNeeded}</p>
  return <div className="space-y-4">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><p className="text-sm text-muted-foreground">{t.description}</p><p className="text-xs text-muted-foreground">{t.automatic}</p></div>{canWrite && <Button onClick={() => setReport(true)}>{t.report}</Button>}</div>
    {report && <Report onAuthError={onAuthError} token={token} project={project} lang={lang} onCancel={() => setReport(false)} onCreated={ticket => { setReport(false); update({ bug: ticket.id }); refresh() }} />}
    <div className="flex flex-wrap gap-2" role="group" aria-label={t.title}>{BUG_VIEWS.map(key => <Button key={key} variant={view === key ? 'default' : 'outline'} aria-pressed={view === key} onClick={() => filter({ view: key })}>{label(key)}</Button>)}</div>
    <div className="grid gap-2 md:grid-cols-2"><input className={fieldClass} aria-label={t.search} placeholder={t.search} value={search} onChange={e => filter({ search: e.target.value }, true)} /><select className={fieldClass} aria-label={t.severity} value={severity} onChange={e => filter({ severity: e.target.value })}><option value="">{t.severity}: {t.all}</option>{BUG_SEVERITIES.map(key => <option key={key} value={key}>{label(key)}</option>)}</select></div>
    <div className="grid min-w-0 gap-2 md:grid-cols-3"><input className={fieldClass} aria-label={t.status} placeholder={t.anyState} value={state} onChange={e => filter({ state: e.target.value }, true)} /><input className={fieldClass} aria-label={t.assignee} placeholder={t.anyAssignee} value={assignee} onChange={e => filter({ assignee: e.target.value }, true)} /><select className={fieldClass} aria-label={t.researchStatus} value={researchStatus} onChange={e => filter({ researchStatus: e.target.value })}><option value="">{t.researchStatus}: {t.all}</option>{RESEARCH_STATUSES.map(key => <option key={key} value={key}>{key === 'none' ? t.noResearch : label(key)}</option>)}</select></div>
    {error && <p role="alert" className="break-words text-sm text-destructive">{error} <Button variant="outline" onClick={refresh}>{t.refresh}</Button></p>}
    {notice && <p role="status" data-testid="linked-notice" className="break-words text-sm text-muted-foreground">{t.linkedUnavailable} {notice}</p>}
    <div className="grid min-w-0 gap-4 md:grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)]">
      <section className="min-w-0 space-y-2" aria-label={t.title}>
        {!page && !error && <p role="status">{t.loading}</p>}
        {page?.items.length === 0 && <p className="text-muted-foreground">{t.empty}</p>}
        {page?.items.map(item => <button key={item.ticket.id} className={`block w-full min-w-0 rounded-xl border p-3 text-left hover:bg-muted ${selected === item.ticket.id ? 'border-primary bg-muted' : 'bg-card'}`} aria-pressed={selected === item.ticket.id} onClick={() => { setBug(item); setNotice(''); update({ bug: item.ticket.id }) }}><span className="text-xs text-muted-foreground">{item.ticket.id}</span><span className="block break-words font-medium">{item.ticket.title}</span><span className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground"><span>{t.severity}: {label(item.severity)}</span><span>{t.triage}: {label(item.triage)}</span><span>{t.status}: {item.ticket.state}</span>{item.latest_job && <span>{t.researchStatus}: {label(item.latest_job.status)}</span>}</span></button>)}
        {page && <div className="flex flex-wrap items-center gap-2 text-sm"><Button variant="outline" disabled={offset === 0} onClick={() => update({ offset: Math.max(0, offset - PAGE_SIZE) })}>{t.previous}</Button><span>{page.total ? offset + 1 : 0}–{offset + page.items.length} / {page.total}</span><Button variant="outline" disabled={offset + page.items.length >= page.total} onClick={() => update({ offset: offset + PAGE_SIZE })}>{t.next}</Button></div>}
      </section>
      {bug ? <Detail key={bug.ticket.id} bug={bug} token={token} lang={lang} canWrite={canWrite} canReview={canReview} canConfigure={canConfigure} refresh={refresh} onAuthError={onAuthError} /> : <p className="text-sm text-muted-foreground">{t.select}</p>}
    </div>
  </div>
}
