import { useEffect, useState } from 'react'
import { api } from '@/lib/api'
import type { EmbeddingSettings as Config } from '@/lib/hybrid-search'
import type { Locale } from '@/lib/i18n'

export function EmbeddingSettings({ token, locale, allowed }: { token: string; locale: Locale; allowed: boolean }) {
  const de = locale === 'de'
  const [saved, setSaved] = useState<Config | null>(null)
  const [draft, setDraft] = useState<Config | null>(null)
  const [key, setKey] = useState('')
  const [clearKey, setClearKey] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!allowed) return
    const controller = new AbortController()
    api<Config>(token, '/settings/embeddings', { signal: controller.signal }).then(config => {
      if (!controller.signal.aborted) { setSaved(config); setDraft(config) }
    }).catch((e: Error) => { if (!controller.signal.aborted) setError(e.message) })
    return () => controller.abort()
  }, [token, allowed])
  if (!allowed) return <p>{de ? 'Nur uneingeschränkte Administratoren können die globale Suche konfigurieren.' : 'Only unrestricted administrators can configure global search.'}</p>
  const changedDestination = saved && draft && (saved.provider !== draft.provider || saved.endpoint !== draft.endpoint)
  const fieldClass = 'mt-1 block min-h-10 w-full rounded-md border bg-background px-3 py-2 text-sm'
  return <section className="space-y-4">
    <div><h2 className="text-lg font-semibold">{de ? 'Dokumentsuche' : 'Document search'}</h2><p className="text-sm text-muted-foreground">{de ? 'Globale Einstellungen für Bedeutungssuche. Die Stichwortsuche funktioniert auch ohne Anbieter.' : 'Global settings for meaning search. Keyword search works without a provider.'}</p></div>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {notice && <p role="status" className="text-sm">{notice}</p>}
    {!draft ? <p>{de ? 'Einstellungen werden geladen…' : 'Loading settings…'}</p> : <form className="space-y-4" onSubmit={event => {
      event.preventDefault()
      if (draft.max_wait_seconds < draft.quiet_seconds) { setError(de ? 'Die maximale Wartezeit muss mindestens der Ruhezeit entsprechen.' : 'Maximum wait must be at least the quiet period.'); return }
      setBusy(true); setError(''); setNotice('')
      const fields = { provider: draft.provider, endpoint: draft.endpoint, model: draft.model, dimensions: draft.dimensions, quiet_seconds: draft.quiet_seconds, max_wait_seconds: draft.max_wait_seconds }
      const api_key = clearKey ? '' : key || undefined
      void api<Config>(token, '/settings/embeddings', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...fields, ...(api_key !== undefined ? { api_key } : {}) }) })
        .then(config => { setSaved(config); setDraft(config); setKey(''); setClearKey(false); setNotice(de ? 'Sucheinstellungen gespeichert.' : 'Search settings saved.') })
        .catch((e: Error) => setError(e.message)).finally(() => setBusy(false))
    }}>
      <label className="block text-sm font-medium">{de ? 'Anbieter' : 'Provider'}<select className={fieldClass} value={draft.provider} onChange={event => {
        const provider = event.target.value as Config['provider']
        setDraft({ ...draft, provider, endpoint: provider === 'voyage' ? 'https://api.voyageai.com/v1/embeddings' : 'https://api.openai.com/v1/embeddings', model: provider === 'voyage' ? 'voyage-4-lite' : '', dimensions: provider === 'voyage' ? 1024 : 1536 })
        setKey(''); setClearKey(false)
      }}><option value="voyage">Voyage AI</option><option value="openai">OpenAI / {de ? 'kompatibler Anbieter' : 'compatible provider'}</option></select></label>
      <label className="block text-sm font-medium">Endpoint<input className={fieldClass} type="url" required value={draft.endpoint} onChange={event => setDraft({ ...draft, endpoint: event.target.value })} /></label>
      <div className="grid gap-4 sm:grid-cols-2"><label className="text-sm font-medium">{de ? 'Modell' : 'Model'}<input className={fieldClass} required value={draft.model} onChange={event => setDraft({ ...draft, model: event.target.value })} /></label>
      <label className="text-sm font-medium">{de ? 'Dimensionen' : 'Dimensions'}<input className={fieldClass} type="number" min={1} required value={draft.dimensions} onChange={event => setDraft({ ...draft, dimensions: Number(event.target.value) })} /></label></div>
      <label className="block text-sm font-medium">API key<input className={fieldClass} type="password" autoComplete="new-password" value={key} disabled={clearKey} onChange={event => setKey(event.target.value)} placeholder={saved?.configured && !changedDestination ? (de ? 'Leer lassen, um vorhandenen Schlüssel zu behalten' : 'Leave blank to keep the existing key') : (de ? 'Neuen Schlüssel eingeben' : 'Enter a new key')} /></label>
      <p className="text-xs text-muted-foreground">{saved?.configured ? (de ? 'Ein Schlüssel ist gespeichert.' : 'A key is stored.') : (de ? 'Kein Schlüssel konfiguriert.' : 'No key configured.')} {de ? 'Gespeicherte Schlüssel werden nie angezeigt.' : 'Stored keys are never displayed.'}</p>
      {changedDestination && <p className="text-sm">{de ? 'Anbieter oder Endpoint geändert: Ohne neuen Schlüssel wird die Bedeutungssuche deaktiviert. Der bisherige Schlüssel wird nicht übernommen.' : 'Provider or endpoint changed: without a new key, meaning search will be disabled. The previous key will not be reused.'}</p>}
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={clearKey} onChange={event => { setClearKey(event.target.checked); setKey('') }} />{de ? 'Gespeicherten Schlüssel entfernen und Bedeutungssuche deaktivieren' : 'Remove the stored key and disable meaning search'}</label>
      <div className="grid gap-4 sm:grid-cols-2">{(['quiet_seconds', 'max_wait_seconds'] as const).map(field => <label key={field} className="text-sm font-medium">{field === 'quiet_seconds' ? (de ? 'Ruhezeit nach Änderungen (Sekunden)' : 'Quiet period after edits (seconds)') : (de ? 'Maximale Wartezeit (Sekunden)' : 'Maximum wait (seconds)')}<input className={fieldClass} type="number" min={1} required value={draft[field]} onChange={event => setDraft({ ...draft, [field]: Number(event.target.value) })} /></label>)}</div>
      <p className="text-xs text-muted-foreground">{de ? 'Änderungen werden nach der Ruhezeit gebündelt, spätestens nach der maximalen Wartezeit. „Dokument synchronisieren“ in der Suche startet die Aktualisierung sofort.' : 'Edits are batched after the quiet period, bounded by the maximum wait. “Sync document” in search schedules an immediate update.'}</p>
      <button type="submit" disabled={busy} className="min-h-10 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-50">{busy ? (de ? 'Wird gespeichert…' : 'Saving…') : (de ? 'Speichern' : 'Save')}</button>
    </form>}
  </section>
}
