import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { api } from '@/lib/api'
import { getRepository, setRepository, extractionRuns, startExtraction, githubWrite, type RepositorySelection, type ExtractionRun } from '@/lib/github'
import { RepositoryFields } from './RepositoryFields'
export function ProjectRepository({ token, project, locale, allowed }: { token: string; project: string; locale: string; allowed: boolean }) {
  const de = locale === 'de'
  const [selection, setSelection] = useState<RepositorySelection | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [runs, setRuns] = useState<ExtractionRun[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const request = useRef(crypto.randomUUID())
  useEffect(() => {
    let live = true
    void getRepository(token, project).then(r => { if (live) { setSelection(r.repository); setLoaded(true) } }).catch((e: Error) => { if (live) setError(e.message) })
    const poll = () => { void extractionRuns(token, project).then(r => { if (live) setRuns(r.items) }).catch((e: Error) => { if (live) setError(e.message) }) }
    poll(); const timer = setInterval(poll, 5000)
    return () => { live = false; clearInterval(timer) }
  }, [token, project])
  async function save(launch: boolean) {
    if (!selection) return
    setBusy(true); setError(''); setSaved(false)
    try {
      await setRepository(token, project, selection); setSaved(true)
      if (launch) {
        const listed = await api<{ items: { id: string }[] }>(token, `/mindmaps?project=${encodeURIComponent(project)}&limit=1`)
        const map = listed.items[0] ?? (await githubWrite<{ mindmap: { id: string } }>(token, '/mindmaps', { project, title: project })).mindmap
        await startExtraction(token, project, map.id, request.current)
        request.current = crypto.randomUUID()
        setRuns((await extractionRuns(token, project)).items)
      }
    } catch (e) { setError((e as Error).message) } finally { setBusy(false) }
  }
  const active = runs.some(r => r.status === 'queued' || r.status === 'running')
  return <section className="border-border flex min-w-0 flex-col gap-3 rounded-lg border p-4">
    <h2 className="font-semibold">{de ? 'Repository und Extraktion' : 'Repository and extraction'}</h2>
    <p className="text-muted-foreground text-sm">{de ? 'Erstelle einen unbestätigten Entwurf in einer leeren Spezifikation. Prüfe die Abschnitte und Quellen im Dokument oder in der Mindmap.' : 'Generate an unconfirmed draft in an empty specification. Review its sections and sources in your document or mindmap.'}</p>
    {allowed && loaded ? <RepositoryFields key={project} token={token} locale={locale} value={selection} onChange={v => { setSelection(v); setSaved(false); request.current = crypto.randomUUID() }} /> : selection && <p className="text-sm break-words">{selection.full_name} · {selection.scope.include.join(', ')}</p>}
    {allowed && <>
      <p className="text-muted-foreground text-sm">{de ? 'Ein Start erlaubt einen kostenpflichtigen Codex-Lauf: maximal 20 Dateien, 100 KB Quelltext und 3 Abschnitte. Bestehende Abschnitte werden nicht überschrieben.' : 'Starting authorizes one Codex run that may incur costs: up to 20 files, 100 KB of source and 3 sections. Existing sections are never overwritten.'}</p>
      <div className="flex flex-wrap gap-2"><Button variant="secondary" disabled={busy || !selection} onClick={() => void save(false)}>{de ? 'Verbindung speichern' : 'Save connection'}</Button><Button disabled={busy || !selection || active} onClick={() => void save(true)}>{de ? 'Speichern und Extraktion starten' : 'Save and start extraction'}</Button></div>
    </>}
    {saved && <p role="status" className="text-sm">{de ? 'Repository-Verbindung gespeichert.' : 'Repository connection saved.'}</p>}
    {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
    {runs.map(r => <div key={r.id} className="bg-muted rounded-md p-3 text-sm"><strong>{({ queued: de ? 'Wartet auf Agent-Dienst' : 'Waiting for agent service', running: de ? 'Code wird gelesen und Entwurf erstellt' : 'Reading code and drafting', completed: de ? 'Bereit zur Prüfung' : 'Ready for review', failed: de ? 'Extraktion fehlgeschlagen' : 'Extraction failed' })[r.status]}</strong>{r.error && <p>{r.error}</p>}<div className="mt-1 flex gap-3"><a className="text-primary underline" href={`/projects/${encodeURIComponent(project)}/specification?view=document`}>{de ? 'Dokument öffnen' : 'Open document'}</a><a className="text-primary underline" href={`/projects/${encodeURIComponent(project)}/specification?view=map`}>{de ? 'Mindmap öffnen' : 'Open mindmap'}</a></div></div>)}
  </section>
}
