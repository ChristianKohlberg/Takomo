import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { useSpecification } from '../specification/context'
import { Button } from '@/components/ui/button'
import { CheckDialog } from '@/components/verification/CheckDialog'
import { CodeReferences } from '@/components/verification/CodeReferences'
import { CaseDetails } from '@/components/verification/CaseDetails'
import { api } from '@/lib/api'
import { pick } from '@/lib/i18n'
import { listInitiatives } from '@/lib/initiatives'
import { archiveCheck, createCheck, listEnvironments, type Environment } from '@/lib/verification'
import { listDefinitions, type TestDefinition, type TestRun, type RunPage } from '@/lib/test-runs'
import { RunComposer } from './RunComposer'
import { RunDetail } from './RunDetail'
import { STR } from './strings'

const states: Record<string, [string, string]> = {
  not_executed: ['Not run', 'Nicht ausgeführt'], verified: ['Verified', 'Verifiziert'],
  failed: ['Failed', 'Fehlgeschlagen'], outdated: ['Needs revalidation', 'Erneut prüfen'],
  in_progress: ['In progress', 'In Bearbeitung'], needs_approval: ['Needs approval', 'Bestätigung ausstehend'],
  mixed_versions: ['Different code versions', 'Verschiedene Codeversionen'],
}
export function TestsView({ compact = false }: { compact?: boolean }) {
  const { token, lang, project, scopes, nodes, checks, editCheck, refreshChecks, onError } = useSpecification()
  const de = lang === 'de'
  const t = pick(STR, lang)
  const [params, setParams] = useSearchParams()
  const tab = params.get('tests') === 'runs' && !compact ? 'runs' : 'definitions'
  const selectedRun = params.get('run')
  const [section, setSection] = useWorkspaceSection()
  const [definitions, setDefinitions] = useState<TestDefinition[]>([])
  const [runs, setRuns] = useState<RunPage>({ items: [], next_cursor: null, total: 0 })
  const [run, setRun] = useState<TestRun | null>(null)
  const [environments, setEnvironments] = useState<Environment[]>([])
  const [initiatives, setInitiatives] = useState<{ id: string; title: string }[]>([])
  const [creating, setCreating] = useState(false)
  const [compose, setCompose] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState(false)
  const epoch = useRef(0)
  const refresh = useCallback(async () => {
    const attempt = ++epoch.current
    try {
      const [defs, page, envs, inis, detail] = await Promise.all([
        listDefinitions(token, project),
        api<RunPage>(token, `/projects/${encodeURIComponent(project)}/test-runs`),
        listEnvironments(token, project), listInitiatives(token, { project }),
        selectedRun ? api<TestRun>(token, `/test-runs/${encodeURIComponent(selectedRun)}`) : Promise.resolve(null),
      ])
      if (attempt !== epoch.current) return
      setDefinitions(defs); setRuns(page); setEnvironments(envs.items.filter(e => !e.archived_at)); setInitiatives(inis.items)
      setRun(detail?.project === project ? detail : null)
      setLoadError(false)
    } catch (error) { if (attempt === epoch.current) { onError(error); setLoadError(true); setRun(null) } }
    finally { if (attempt === epoch.current) setLoading(false) }
  }, [token, project, selectedRun, onError])
  useEffect(() => { const counter = epoch; void refresh(); return () => { counter.current++ } }, [refresh])
  useProjectUpdates(token, project, refresh)
  const navigateTab = (next: string, id?: string) => setParams(current => {
    const nextParams = new URLSearchParams(current)
    nextParams.set('tests', next)
    if (next === 'runs') { nextParams.set('view', 'tests'); nextParams.delete('panel'); nextParams.delete('check') }
    if (id) nextParams.set('run', id); else nextParams.delete('run')
    return nextParams
  })
  const changed = (next: TestRun) => { setRun(next); setCompose(null); if (tab !== 'runs' || selectedRun !== next.id) navigateTab('runs', next.id); void refresh(); void refreshChecks() }
  const filtered = definitions.filter(d => !section || d.definition.node === section)
  const selectedChecks = new Set(filtered.map(d => d.id))
  const visibleRuns = runs.items.filter(r => !section || r.checks.some(c => selectedChecks.has(c)))
  return <>
    <div className="flex flex-none flex-wrap items-center gap-2 border-b px-4 py-2">
      {!compact && <div className="flex gap-1" aria-label={de ? 'Tests' : 'Tests'}>
        <Button variant={tab === 'definitions' ? 'secondary' : 'ghost'} aria-pressed={tab === 'definitions'} onClick={() => navigateTab('definitions')}>{de ? 'Definitionen' : 'Definitions'}</Button>
        <Button variant={tab === 'runs' ? 'secondary' : 'ghost'} aria-pressed={tab === 'runs'} onClick={() => navigateTab('runs')}>{de ? 'Testläufe' : 'Runs'}</Button>
      </div>}
      <span className="grow" />
      {tab === 'definitions' && scopes.includes('write') && <Button onClick={() => setCreating(true)}>+ {de ? 'Test definieren' : 'Define test'}</Button>}
      <Button variant="outline" onClick={() => void refresh()}>{de ? 'Aktualisieren' : 'Refresh'}</Button>
    </div>
    <main className="min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-5">
      <div className="mx-auto grid w-full max-w-240 gap-4 pb-12">
        <div><h1 className="text-lg font-semibold">{tab === 'definitions' ? (de ? 'Was muss stimmen?' : 'What must be true?') : (de ? 'Was wurde getestet?' : 'What was tested?')}</h1>
          <p className="text-sm text-muted-foreground">{tab === 'definitions' ? (de ? 'Gemeinsame Definitionen beschreiben das erwartete Verhalten. Jeder Testlauf hält den geprüften Stand fest.' : 'Shared definitions describe expected behavior. Each run captures the revisions it tests.') : (de ? 'Ausführung, Nachweise und menschliche Prüfung je Versuch.' : 'Execution, evidence, and human review for each attempt.')}</p></div>
        {section && !compact && <div className="flex flex-wrap items-center gap-2 rounded-md bg-muted px-3 py-2 text-sm"><span className="min-w-0 break-words">{nodes.find(n => n.id === section)?.title ?? section}</span><Button variant="ghost" size="sm" onClick={() => setSection(null)}>{de ? 'Alle Abschnitte' : 'All sections'}</Button></div>}
        {loading && <p role="status">{de ? 'Laden …' : 'Loading …'}</p>}
        {loadError && <p role="alert">{de ? 'Daten konnten nicht geladen werden. Bitte erneut aktualisieren.' : 'Could not load data. Refresh to try again.'}</p>}
        {tab === 'definitions' ? <>
          {!loading && !filtered.length && <div className="rounded-xl border border-dashed px-5 py-8 text-center text-sm text-muted-foreground">{de ? 'Noch keine Testdefinitionen für diesen Bereich.' : 'No test definitions for this scope yet.'}</div>}
          {filtered.map(d => <article key={d.id} className="min-w-0 rounded-xl border bg-card p-4">
            <div className="flex flex-wrap items-start justify-between gap-3"><div className="min-w-0 flex-1"><h2 className="break-words font-semibold">{d.definition.title}</h2><p className="mt-1 text-xs text-muted-foreground">{d.definition.layer} · {d.definition.severity} · {d.definition.cases.length} {de ? 'Fälle' : 'cases'}</p></div>
              <span className={`rounded-md px-2 py-1 text-xs ${d.execution.state === 'verified' ? 'bg-ok-bg text-ok' : d.execution.state === 'failed' ? 'bg-nfbg text-nf' : 'bg-muted text-muted-foreground'}`}>{states[d.execution.state]?.[de ? 1 : 0] ?? d.execution.state}</span></div>
            {d.definition.precondition && <p className="mt-3 whitespace-pre-wrap break-words text-sm text-muted-foreground">{d.definition.precondition}</p>}
            {!d.definition.cases.length && <p className="mt-3 text-sm text-muted-foreground">{de ? 'Noch keine Fälle definiert. Bitte deinen Agenten, Fälle für diese Definition zu erstellen.' : 'No cases defined yet. Ask your agent to generate cases for this definition.'}</p>}
            <details className="mt-3 text-sm"><summary className="cursor-pointer">{de ? 'Testfälle' : 'Test cases'}</summary><ul className="mt-2 grid gap-2">{d.definition.cases.map(c => <li key={c.id} className="min-w-0 rounded-md bg-muted px-3 py-2"><p className="break-words font-medium">{c.label || c.key}</p><CaseDetails assignment={c.assignment} lang={lang} /></li>)}</ul></details>
            <CodeReferences metadata={checks.find(check => check.id === d.id)?.metadata} lang={lang} />
            <div className="mt-4 flex flex-wrap gap-2"><Button variant="outline" size="sm" onClick={() => editCheck(d.id)}>{de ? 'Definition öffnen' : 'Open definition'}</Button>
              {scopes.includes('write') && <Button size="sm" disabled={!d.definition.cases.length} onClick={() => setCompose(d.id)}>{de ? 'Testlauf erstellen' : 'Create run'}</Button>}
              {d.execution.environments.filter(e => e.run).map(e => <Button key={e.run} variant="ghost" size="sm" onClick={() => navigateTab('runs', e.run)}>{environments.find(env => env.id === e.environment)?.name ?? (de ? 'Letzter Lauf' : 'Latest run')} · {states[e.state]?.[de ? 1 : 0] ?? e.state}</Button>)}
              {scopes.includes('write') && <Button variant="ghost" size="sm" onClick={() => { if (window.confirm(t.confirmArchiveCheck)) void archiveCheck(token, d.id).then(async () => { await refresh(); await refreshChecks() }).catch(onError) }}>{de ? 'Archivieren' : 'Archive'}</Button>}
            </div>
          </article>)}
        </> : <>
          {run && <RunDetail key={run.id} run={run} changed={changed} />}
          <div className="grid gap-2">{visibleRuns.map(r => <button key={r.id} type="button" className={`min-w-0 rounded-lg border p-3 text-left hover:bg-muted ${r.id === selectedRun ? 'border-primary' : ''}`} onClick={() => navigateTab('runs', r.id)}>
            <div className="flex flex-wrap justify-between gap-2 text-sm"><span>{r.kind === 'legacy' ? (de ? 'Historischer Nachweis' : 'Legacy evidence') : `${r.case_count} ${de ? 'Fälle' : 'cases'}`} · {r.status}</span><time className="text-xs text-muted-foreground">{new Date(r.created_at).toLocaleString()}</time></div>
            <p className="mt-1 break-all text-xs text-muted-foreground">{r.code_ref ?? '—'} · {r.environment_snapshot?.name ?? r.environment ?? '—'} · {r.id}</p>
          </button>)}</div>
          {!loading && !visibleRuns.length && <p className="py-6 text-center text-sm text-muted-foreground">{de ? 'Keine Testläufe auf dieser Seite.' : 'No runs on this page.'}</p>}
          {runs.next_cursor && <Button variant="outline" onClick={() => {
            const attempt = epoch.current
            void api<RunPage>(token, `/projects/${encodeURIComponent(project)}/test-runs?cursor=${encodeURIComponent(runs.next_cursor!)}`).then(page => { if (attempt === epoch.current) setRuns(current => ({ ...page, items: [...current.items, ...page.items] })) }).catch(onError)
          }}>{de ? 'Ältere Läufe laden' : 'Load older runs'}</Button>}
        </>}
      </div>
    </main>
    <CheckDialog open={creating} onOpenChange={setCreating} initiatives={initiatives} environments={environments.map(e => ({ id: e.id, slug: e.slug }))} nodes={nodes.map(n => ({ id: n.id, title: n.title }))} defaultNode={section ?? undefined} labels={t} onSubmit={async fields => { await createCheck(token, project, fields); await refresh(); await refreshChecks() }} />
    {compose && <RunComposer key={compose} check={compose} environments={environments} close={() => setCompose(null)} created={changed} />}
  </>
}
