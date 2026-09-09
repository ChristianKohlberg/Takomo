import { useSettingsDraft } from './SettingsDrafts'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { getClassificationPolicy, saveClassificationPolicy, classifyProjectDocuments, type ClassificationPolicy } from '@/lib/ticket-document-links'
import { DOCUMENT_LINKS } from '@/components/board/document-link-strings'
interface Props { token: string; project: string; lang: Locale; readOnly: boolean; canClassify?: boolean }
export function DocumentClassificationPolicy(props: Props) { return <Policy key={`${props.token}:${props.project}`} {...props} /> }
function Policy({ token, project, lang, readOnly, canClassify }: Props) {
  const t = DOCUMENT_LINKS[lang]
  const retryId = useRef<string | null>(null)
  const [scheduled, setScheduled] = useState<number | null>(null)
  const [original, setOriginal] = useState<ClassificationPolicy>('suggest')
  const [mode, setMode] = useState<ClassificationPolicy>('suggest')
  const [ready, setReady] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [saved, setSaved] = useState(false)
  const [refresh, setRefresh] = useState(0); const writer = useRef<AbortController | null>(null)
  useSettingsDraft((ready && mode !== original) || busy)
  useEffect(() => () => writer.current?.abort(), [])
  useEffect(() => {
    const controller = new AbortController()
    getClassificationPolicy(token, project, controller.signal).then(value => { if (!controller.signal.aborted) { setMode(value.mode); setOriginal(value.mode); setReady(true) } }).catch(cause => { if (!controller.signal.aborted) setError(String(cause)) })
    return () => controller.abort()
  }, [token, project, refresh])
  async function save() {
    if (readOnly || writer.current || !ready) return
    const controller = new AbortController(); writer.current = controller; setBusy(true); setError(''); setSaved(false)
    try { await saveClassificationPolicy(token, project, mode, controller.signal); if (!controller.signal.aborted) { setSaved(true); setOriginal(mode) } }
    catch (cause) { if (!controller.signal.aborted) setError(String(cause)) }
    finally { if (!controller.signal.aborted) { setBusy(false); writer.current = null } }
  }
  return <section className="space-y-3 rounded border border-border-soft p-4" aria-label={t.policy}>
    <h3 className="font-semibold">{t.policy}</h3><p className="text-sm text-muted-foreground">{t.policyHint}</p>
    <select aria-label={t.policy} value={mode} disabled={readOnly || busy || !ready} onChange={event => { setMode(event.target.value as ClassificationPolicy); setSaved(false) }} className="w-full rounded border border-border-soft bg-background p-2 text-sm"><option value="suggest">{t.suggest}</option><option value="auto_apply_clear">{t.auto}</option></select>
    {!readOnly && <Button size="sm" disabled={busy || !ready} onClick={() => void save()}>{t.save}</Button>}
    {canClassify && <Button size="sm" variant="outline" disabled={busy || !ready} onClick={async () => {
      if (writer.current) return
      const controller = new AbortController(); writer.current = controller; setBusy(true); setError(''); retryId.current ??= crypto.randomUUID()
      try { const result = await classifyProjectDocuments(token, project, retryId.current, controller.signal); if (!controller.signal.aborted) { setScheduled(result.scheduled); retryId.current = null } }
      catch (cause) { if (!controller.signal.aborted) setError(String(cause)) }
      finally { if (!controller.signal.aborted) { writer.current = null; setBusy(false) } }
    }}>{t.backfill}</Button>}
    {scheduled !== null && <p role="status" className="text-sm">{t.scheduled.replace('{n}', String(scheduled))}</p>}
    {saved && <p role="status" className="text-sm">{t.saved}</p>}{error && <div role="alert"><p className="text-sm text-destructive">{error}</p>{!ready && <Button size="sm" onClick={() => { setError(''); setRefresh(value => value + 1) }}>{t.refresh}</Button>}</div>}
  </section>
}
