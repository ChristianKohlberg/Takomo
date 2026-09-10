import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { githubStatus, githubRepositories, type Installation, type Repository, type RepositorySelection } from '@/lib/github'
export function RepositoryFields({ token, locale, value, onChange }: { token: string; locale: string; value: RepositorySelection | null; onChange: (value: RepositorySelection | null) => void }) {
  const de = locale === 'de'
  const [connections, setConnections] = useState<Installation[]>([])
  const [installation, setInstallation] = useState(value?.installation ?? 0)
  const [repos, setRepos] = useState<Repository[]>([])
  const [page, setPage] = useState(1)
  const [total, setTotal] = useState(0)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  useEffect(() => { let live = true; void githubStatus(token).then(s => { if (live) setConnections(s.connections) }).catch((e: Error) => { if (live) setError(e.message) }); return () => { live = false } }, [token])
  useEffect(() => {
    if (!installation) return
    let live = true; setLoading(true); setError('')
    void githubRepositories(token, installation, page).then(r => { if (live) { setRepos(r.items); setTotal(r.total) } }).catch((e: Error) => { if (live) { setRepos([]); setError(e.message) } }).finally(() => { if (live) setLoading(false) })
    return () => { live = false }
  }, [token, installation, page])
  return <div className="flex min-w-0 flex-col gap-3">
    <a href="/settings?section=github" target="_blank" rel="noreferrer" className="text-primary text-sm underline">{de ? 'GitHub verbinden oder Zugriff ändern' : 'Connect GitHub or change access'} ↗</a>
    <Button variant="ghost" onClick={() => { setError(''); void githubStatus(token).then(s => setConnections(s.connections)).catch((e: Error) => setError(e.message)) }}>{de ? 'Verbindungen aktualisieren' : 'Refresh connections'}</Button>
    <Field label={de ? 'GitHub-Konto' : 'GitHub account'}>{id => <select id={id} className="border-border bg-background w-full rounded-md border p-2" value={installation} onChange={e => { setInstallation(Number(e.target.value)); setPage(1); setRepos([]); onChange(null) }}><option value={0}>{de ? 'Konto auswählen' : 'Choose account'}</option>{connections.map(c => <option key={c.id} value={c.id}>{c.account}</option>)}</select>}</Field>
    {installation > 0 && <>
      <Field label="Repository">{id => <select id={id} disabled={loading} className="border-border bg-background w-full rounded-md border p-2" value={value?.repository ?? 0} onChange={e => { const r = repos.find(r => r.id === Number(e.target.value)); onChange(r ? { installation, repository: r.id, full_name: r.full_name, scope: value?.scope ?? { include: [''], exclude: [] } } : null) }}><option value={0}>{loading ? (de ? 'Lädt…' : 'Loading…') : (de ? 'Repository auswählen' : 'Choose repository')}</option>{repos.map(r => <option key={r.id} value={r.id}>{r.full_name}{r.private ? ' · private' : ''}</option>)}</select>}</Field>
      {total > 100 && <div className="flex gap-2"><Button variant="ghost" disabled={page === 1 || loading} onClick={() => { setPage(p => p - 1); onChange(null) }}>{de ? 'Zurück' : 'Previous'}</Button><span>{page} / {Math.ceil(total / 100)}</span><Button variant="ghost" disabled={page * 100 >= total || loading} onClick={() => { setPage(p => p + 1); onChange(null) }}>{de ? 'Weiter' : 'Next'}</Button></div>}
    </>}
    {value && <><Field label={de ? 'Datei oder Ordner für die Extraktion' : 'File or folder to extract'} hint={de ? 'Relativer Pfad, z. B. src/checkout. Für den Test: examples/extraction-fixture. Nur dieser Bereich wird gelesen.' : 'Relative path, e.g. src/checkout. For the sample: examples/extraction-fixture. Only this scope is read.'}>{id => <Input id={id} value={value.scope.include[0] ?? ''} onChange={e => onChange({ ...value, scope: { ...value.scope, include: [e.target.value] } })} />}</Field><Button variant="secondary" onClick={() => onChange({ ...value, scope: { include: ['examples/extraction-fixture/checkout.mjs'], exclude: [] } })}>{de ? 'Testdatei auswählen' : 'Use sample file'}</Button><p className="text-muted-foreground text-xs">{de ? 'Die Testdatei ist im Takomo-Repository enthalten. Wähle dieses Repository oder einen Fork mit der Testdatei. Dies startet noch keine Extraktion.' : 'The sample is included in the Takomo repository. Select that repository or a fork containing the sample. This does not start extraction.'}</p></>}
    {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
  </div>
}
