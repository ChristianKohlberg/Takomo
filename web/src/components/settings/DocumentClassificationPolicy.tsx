import { useSettingsDraft } from './SettingsDrafts'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { getClassificationPolicy, saveClassificationPolicy, classifyProjectDocuments, type ClassificationPolicy, type ClassificationScheduling } from '@/lib/ticket-document-links'
import { DOCUMENT_LINKS } from '@/components/board/document-link-strings'
interface Props { token: string; project: string; lang: Locale; readOnly: boolean; canClassify?: boolean }
export function DocumentClassificationPolicy(props: Props) { return <Policy key={`${props.token}:${props.project}`} {...props} /> }
function Policy({ token, project, lang, readOnly, canClassify }: Props) {
  const t = DOCUMENT_LINKS[lang]
  const labels = lang === 'de' ? {
    scheduling: 'Zuordnung starten', off: 'Aus', manual: 'Manuell', automatic: 'Automatisch',
    hint: 'Automatisch: neue und wesentlich bearbeitete Tickets zuordnen. Manuell: nur auf ausdrückliche Anfrage. Aus: keine neuen Zuordnungen.',
    consequence: 'Manuell storniert wartende automatische Aufträge. Aus storniert alle wartenden Zuordnungen. Laufende Aufträge werden abgeschlossen; bestehende Verknüpfungen bleiben erhalten.',
    cancelled: '{n} wartende Aufträge storniert.',
    saveFirst: 'Änderungen vor dem Start einer Zuordnung speichern.',
  } : {
    scheduling: 'Classification scheduling', off: 'Off', manual: 'Manual', automatic: 'Automatic',
    hint: 'Automatic: classify new and materially edited tickets. Manual: only on explicit request. Off: no new classifications.',
    consequence: 'Manual cancels queued automatic jobs. Off cancels all queued classifications. Running jobs finish; existing links are preserved.',
    cancelled: '{n} queued jobs cancelled.',
    saveFirst: 'Save changes before starting classification.',
  }
  const [scheduling, setScheduling] = useState<ClassificationScheduling>('automatic')
  const [originalScheduling, setOriginalScheduling] = useState<ClassificationScheduling>('automatic')
  const retryId = useRef<string | null>(null)
  const [cancelled, setCancelled] = useState(0)
  const [scheduled, setScheduled] = useState<number | null>(null)
  const [original, setOriginal] = useState<ClassificationPolicy>('suggest')
  const [mode, setMode] = useState<ClassificationPolicy>('suggest')
  const dirty = mode !== original || scheduling !== originalScheduling
  const [ready, setReady] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [saved, setSaved] = useState(false)
  const [refresh, setRefresh] = useState(0); const writer = useRef<AbortController | null>(null)
  useSettingsDraft((ready && dirty) || busy)
  useEffect(() => () => writer.current?.abort(), [])
  useEffect(() => {
    const controller = new AbortController()
    getClassificationPolicy(token, project, controller.signal).then(value => { if (!controller.signal.aborted) { setMode(value.mode); setOriginal(value.mode); setScheduling(value.scheduling ?? 'automatic'); setOriginalScheduling(value.scheduling ?? 'automatic'); setReady(true) } }).catch(cause => { if (!controller.signal.aborted) setError(String(cause)) })
    return () => controller.abort()
  }, [token, project, refresh])
  async function save() {
    if (readOnly || writer.current || !ready) return
    const controller = new AbortController(); writer.current = controller; setBusy(true); setError(''); setSaved(false)
    try { const result = await saveClassificationPolicy(token, project, mode, controller.signal, scheduling); if (!controller.signal.aborted) { setSaved(true); setCancelled(result.cancelled ?? 0); setScheduled(null); setOriginal(mode); setOriginalScheduling(scheduling) } }
    catch (cause) { if (!controller.signal.aborted) setError(String(cause)) }
    finally { if (!controller.signal.aborted) { setBusy(false); writer.current = null } }
  }
  return <section className="space-y-3 rounded border border-border-soft p-4" aria-label={t.policy}>
    <h3 className="font-semibold">{t.policy}</h3><p className="text-sm text-muted-foreground">{t.policyHint}</p>
    <label className="block space-y-1 text-sm"><span>{labels.scheduling}</span><select aria-label={labels.scheduling} value={scheduling} disabled={readOnly || busy || !ready} onChange={event => { setScheduling(event.target.value as ClassificationScheduling); setSaved(false) }} className="w-full rounded border border-border-soft bg-background p-2 text-sm"><option value="off">{labels.off}</option><option value="manual">{labels.manual}</option><option value="automatic">{labels.automatic}</option></select></label>
    <p className="text-sm text-muted-foreground">{labels.hint}</p><p className="text-sm text-muted-foreground">{labels.consequence}</p>
    <select aria-label={t.policy} value={mode} disabled={readOnly || busy || !ready} onChange={event => { setMode(event.target.value as ClassificationPolicy); setSaved(false) }} className="w-full rounded border border-border-soft bg-background p-2 text-sm"><option value="suggest">{t.suggest}</option><option value="auto_apply_clear">{t.auto}</option></select>
    {!readOnly && <Button size="sm" disabled={busy || !ready} onClick={() => void save()}>{t.save}</Button>}
    {canClassify && <Button size="sm" variant="outline" disabled={busy || !ready || dirty || originalScheduling === 'off'} onClick={async () => {
      if (writer.current) return
      const controller = new AbortController(); writer.current = controller; setBusy(true); setError(''); retryId.current ??= crypto.randomUUID()
      try { const result = await classifyProjectDocuments(token, project, retryId.current, controller.signal); if (!controller.signal.aborted) { setScheduled(result.scheduled); retryId.current = null } }
      catch (cause) { if (!controller.signal.aborted) setError(String(cause)) }
      finally { if (!controller.signal.aborted) { writer.current = null; setBusy(false) } }
    }}>{t.backfill}</Button>}
    {canClassify && dirty && <p className="text-sm text-muted-foreground">{labels.saveFirst}</p>}
    {scheduled !== null && <p role="status" className="text-sm">{t.scheduled.replace('{n}', String(scheduled))}</p>}
    {saved && cancelled > 0 && <p role="status" className="text-sm">{labels.cancelled.replace('{n}', String(cancelled))}</p>}
    {saved && <p role="status" className="text-sm">{t.saved}</p>}{error && <div role="alert"><p className="text-sm text-destructive">{error}</p>{!ready && <Button size="sm" onClick={() => { setError(''); setRefresh(value => value + 1) }}>{t.refresh}</Button>}</div>}
  </section>
}
