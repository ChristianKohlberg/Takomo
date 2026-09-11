import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { api } from '@/lib/api'

type Quota = { used_percent: number; resets_at: number | null }
type Connection = { id: string; service_id: string; projects: string[] | '*'; seen_at: number; busy: boolean; action: string | null; command_id: string | null; expires_at: number | null; report: { status: string; account?: { email: string | null; plan: string | null; auth_mode: string } | null; limits?: { primary: Quota | null; secondary: Quota | null } | null; device?: { verification_url: string; user_code: string } | null; error?: string | null } }
export function CodexSettings({ token, locale, allowed }: { token: string; locale: string; allowed: boolean }) {
  const de = locale === 'de'
  const [items, setItems] = useState<Connection[]>([])
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState('')
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    if (!allowed) return
    const controller = new AbortController()
    let running = false
    const load = async () => {
      if (running || document.visibilityState === 'hidden') return
      running = true
      try { const result = await api<{ items: Connection[] }>(token, '/integrations/codex', { signal: controller.signal }); if (!controller.signal.aborted) { setItems(result.items); setLoaded(true); setError('') } }
      catch (e) { if (!controller.signal.aborted) setError((e as Error).message) }
      finally { running = false }
    }
    void load(); const timer = setInterval(() => void load(), 3000)
    document.addEventListener('visibilitychange', load)
    return () => { controller.abort(); clearInterval(timer); document.removeEventListener('visibilitychange', load) }
  }, [token, allowed, refresh])
  async function command(id: string, action: string) {
    setBusy(id); setError('')
    try {
      await api(token, `/integrations/codex/${encodeURIComponent(id)}`, { method: 'POST', body: JSON.stringify({ action, request_id: crypto.randomUUID() }) })
      setRefresh(n => n + 1)
    } catch (e) { setError((e as Error).message) } finally { setBusy('') }
  }
  if (!allowed) return <p>{de ? 'Nur Administratoren mit Zugriff auf alle Projekte können Codex-Verbindungen verwalten.' : 'Only administrators with access to all projects can manage Codex connections.'}</p>
  const status = (value: string) => ({ connected: de ? 'Verbunden' : 'Connected', disconnected: de ? 'Nicht verbunden' : 'Disconnected', unknown: de ? 'Noch nicht geprüft' : 'Not checked yet', login_pending: de ? 'Anmeldung bestätigen' : 'Approve sign-in', error: de ? 'Verbindung prüfen' : 'Connection needs attention' })[value] ?? value
  return <section className="flex min-w-0 flex-col gap-4">
    <h2 className="text-lg font-semibold">{de ? 'KI-Verbindungen · Codex' : 'AI connections · Codex'}</h2>
    <p className="text-muted-foreground text-sm">{de ? 'Verbinde das Konto, das dieser Worker für die zugewiesenen Projekte verwendet. Änderungen werden erst ausgeführt, wenn der Worker frei ist. Zugangsdaten bleiben beim Worker.' : 'Connect the account this worker uses for its assigned projects. Changes take effect when the worker is idle. Credentials stay on the worker.'}</p>
    {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
    {!loaded && !error && <p role="status">{de ? 'Lädt…' : 'Loading…'}</p>}
    {loaded && !items.length && <p className="text-muted-foreground text-sm">{de ? 'Noch kein Worker registriert. Ein Betreiber muss die aktuelle Worker-Version mit TAKOMO_CODEX_CONNECTIONS=1 starten.' : 'No worker registered yet. An operator must start the updated worker with TAKOMO_CODEX_CONNECTIONS=1.'}</p>}
    {items.map(c => <article key={c.id} className="border-border min-w-0 rounded-lg border p-4">
      <h3 className="break-all font-medium">{c.service_id}</h3>
      <p className="mt-1 text-sm">{status(c.report.status)} · {c.busy ? (de ? 'Worker arbeitet' : 'Worker busy') : Date.now() - c.seen_at > 75000 ? (de ? 'Worker nicht erreichbar' : 'Worker not recently seen') : (de ? 'Worker bereit' : 'Worker available')}</p>
      <p className="text-muted-foreground mt-1 break-words text-xs">{de ? 'Projekte' : 'Projects'}: {c.projects === '*' ? (de ? 'Alle Projekte' : 'All projects') : c.projects.join(', ')}</p>
      <p className="text-muted-foreground mt-1 text-xs">{de ? 'Zuletzt gesehen' : 'Last seen'}: {new Date(c.seen_at).toLocaleString(locale)}</p>
      {c.report.account && <p className="mt-2 break-words text-sm">{c.report.account.email ?? c.report.account.auth_mode}{c.report.account.plan ? ` · ${c.report.account.plan}` : ''}</p>}
      {c.action && <p role="status" className="mt-2 text-sm">{de ? 'Verbindungsanfrage läuft; ein beschäftigter Worker führt sie nach seinem aktuellen Auftrag aus.' : 'Connection request pending; a busy worker will handle it after its current job.'}</p>}
      {c.action === 'login' && c.report.device && c.expires_at != null && c.expires_at > Date.now() && <div className="bg-muted mt-3 rounded-lg p-3">
        <a className="text-primary underline" target="_blank" rel="noopener noreferrer" href={c.report.device.verification_url}>{de ? 'Bei OpenAI anmelden' : 'Sign in on OpenAI'}</a>
        <p className="mt-2 text-sm">{de ? 'Diesen einmaligen Code eingeben' : 'Enter this one-time code'}: <strong className="font-mono">{c.report.device.user_code}</strong></p>
        <p className="text-muted-foreground mt-2 text-xs">{de ? 'Gültig bis' : 'Valid until'}: {new Date(c.expires_at).toLocaleTimeString(locale)}</p>
        <p className="text-muted-foreground mt-2 text-xs">{de ? 'Geräteanmeldung muss ggf. in den ChatGPT-Sicherheitseinstellungen freigegeben werden.' : 'Device login may need enabling in your ChatGPT security settings.'}</p>
      </div>}
      {c.report.error && <p role="alert" className="text-destructive mt-2 text-sm">{c.report.error}</p>}
      {c.report.status === 'connected' && <div className="mt-3 space-y-1 text-sm">
        {[c.report.limits?.primary, c.report.limits?.secondary].map((q, i) => q && <p key={i}>{de ? 'Nutzungslimit' : 'Usage limit'} {i + 1}: {q.used_percent}% {de ? 'verbraucht' : 'used'}{q.resets_at != null ? ` · ${de ? 'Zurückgesetzt am' : 'Resets'} ${new Date(q.resets_at * 1000).toLocaleString(locale)}` : ''}</p>)}
        {!c.report.limits?.primary && !c.report.limits?.secondary && <p className="text-muted-foreground text-xs">{de ? 'Keine Kontolimits gemeldet. Tokenverbrauch einzelner Läufe steht in der Agenten-Queue.' : 'No account limits reported. Per-run token usage is shown in the Agent queue.'}</p>}
      </div>}
      <div className="mt-3 flex flex-wrap gap-2">
        <Button disabled={busy === c.id || !!c.action} onClick={() => void command(c.id, 'login')}>{c.report.status === 'connected' ? (de ? 'Neu verbinden' : 'Reconnect') : (de ? 'ChatGPT / Codex verbinden' : 'Connect ChatGPT / Codex')}</Button>
        <Button variant="secondary" disabled={busy === c.id || !!c.action} onClick={() => void command(c.id, 'refresh')}>{de ? 'Konto und Limits aktualisieren' : 'Refresh account and limits'}</Button>
        <Button variant="outline" disabled={busy === c.id || !!c.action} onClick={() => void command(c.id, 'logout')}>{de ? 'Abmelden' : 'Disconnect'}</Button>
        {c.action && <Button variant="outline" disabled={busy === c.id} onClick={() => void command(c.id, 'cancel')}>{de ? 'Anfrage abbrechen' : 'Cancel request'}</Button>}
      </div>
    </article>)}
  </section>
}
