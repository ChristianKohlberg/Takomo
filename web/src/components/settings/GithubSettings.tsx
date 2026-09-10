import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { githubStatus, githubInstallations, connectGithub, disconnectGithub, type GithubStatus, type Installation } from '@/lib/github'
export function GithubSettings({ token, locale, allowed, onChanged }: { token: string; locale: string; allowed: boolean; onChanged?: () => void }) {
  const de = locale === 'de'
  const [status, setStatus] = useState<GithubStatus | null>(null)
  const [installations, setInstallations] = useState<Installation[]>([])
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function refresh() { setStatus(await githubStatus(token)) }
  useEffect(() => { if (allowed) void refresh().catch((e: Error) => setError(e.message)) }, [token, allowed]) // eslint-disable-line react-hooks/exhaustive-deps
  async function act(f: () => Promise<unknown>) {
    setBusy(true); setError('')
    try { await f(); await refresh(); onChanged?.() } catch (e) { setError((e as Error).message) } finally { setBusy(false) }
  }
  if (!allowed) return <p>{de ? 'GitHub-Verbindungen benötigen einen Administrator mit Zugriff auf alle Projekte.' : 'GitHub connections require an administrator with access to all projects.'}</p>
  return <section className="flex min-w-0 flex-col gap-4">
    <h2 className="text-lg font-semibold">GitHub</h2>
    <p className="text-muted-foreground text-sm">{de ? 'Verbinde ausgewählte Repositorys, um aus bestehendem Code einen ersten Spezifikationsentwurf zu erstellen. Du prüfst und bearbeitest ihn im Dokument oder in der Mindmap.' : 'Connect selected repositories to turn existing code into a first specification draft. Review and edit it in your document or mindmap.'}</p>
    <p className="text-muted-foreground text-sm">{de ? 'Benötigt: Lesezugriff auf Inhalte und Repository-Metadaten. Weitere Repositorys und neue Berechtigungen bestätigst du später auf GitHub. Verbinden startet keine Extraktion.' : 'Required access: read repository contents and metadata. You approve additional repositories and future permission requests on GitHub. Connecting does not start extraction.'}</p>
    {status && !status.configured && <div className="bg-muted rounded-lg p-4 text-sm">
      <p>{de ? 'Die GitHub App ist noch nicht eingerichtet. Ein Betreiber muss eine eigene App für diese Takomo-Instanz registrieren und ihre Zugangsdaten auf dem Server konfigurieren.' : 'The GitHub App is not configured yet. An operator must register a dedicated app for this Takomo instance and configure its credentials on the server.'}</p>
      <p className="mt-2">{de ? 'Danach kannst du hier ein Konto verbinden und Repositorys auswählen.' : 'You can then connect an account and select repositories here.'}</p>
    </div>}
    {status?.configured && <>
      <div className="flex flex-wrap gap-2">
        <a className="text-primary underline" href={`https://github.com/apps/${status.app_slug}/installations/new`} target="_blank" rel="noreferrer">{de ? 'GitHub-Konto verbinden' : 'Connect GitHub account'} ↗</a>
        <Button variant="secondary" disabled={busy} onClick={() => void act(async () => { const result = await githubInstallations(token); setInstallations(result.items); if (result.has_more) setError(de ? 'Es werden nur die ersten 100 Installationen angezeigt.' : 'Only the first 100 installations are shown.') })}>{de ? 'Nach der Installation aktualisieren' : 'Refresh after installing'}</Button>
      </div>
      {installations.filter(i => !status.connections.some(c => c.id === i.id)).map(i => <div key={i.id} className="flex flex-wrap items-center gap-3"><span>{i.account}</span><Button disabled={busy || i.suspended} onClick={() => void act(() => connectGithub(token, i.id))}>{de ? 'Verbinden' : 'Connect'}</Button></div>)}
      {status.connections.map(i => <div key={i.id} className="border-border flex flex-wrap items-center gap-3 rounded-lg border p-3">
        <span className="font-medium">{i.account}</span>
        <a className="text-primary underline" href={i.management_url || `https://github.com/apps/${status.app_slug}/installations/new`} target="_blank" rel="noreferrer">{de ? 'Repositorys und Berechtigungen verwalten' : 'Manage repositories and permissions'} ↗</a>
        <Button variant="ghost" disabled={busy} onClick={() => void act(() => disconnectGithub(token, i.id))}>{de ? 'Trennen' : 'Disconnect'}</Button>
      </div>)}
      <p className="text-muted-foreground text-xs">{de ? 'Trennen entfernt die Verbindung in Takomo und stoppt ausstehende Extraktionen. Den Zugriff der App widerrufst du auf GitHub. Bestehende Dokumente bleiben erhalten.' : 'Disconnecting removes the Takomo connection and stops pending extractions. Revoke the app’s access on GitHub. Existing documents are kept.'}</p>
    </>}
    {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
  </section>
}
