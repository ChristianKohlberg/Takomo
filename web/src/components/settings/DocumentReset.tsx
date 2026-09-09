import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { listMindmaps } from '@/lib/mindmaps'
import { listInitiatives } from '@/lib/initiatives'
import { api } from '@/lib/api'
import { defineStrings, pick, type Locale } from '@/lib/i18n'

const STR = defineStrings({
  en: {
    title: 'Reset a document',
    description: 'Clear a document in this project and start again. Choose a document, then confirm twice.',
    search: 'Find a document',
    searchHint: 'Search by title. Reset affects only the selected document in this project.',
    document: 'Document',
    choose: 'Choose a document…',
    collaborative: 'Specification',
    initiative: 'Initiative',
    loading: 'Loading documents…',
    empty: 'No active documents match this search.',
    limited: 'More documents match. Narrow the search to find the document you want.',
    retry: 'Retry',
    reset: 'Reset document…',
    first: 'Clear this document? (1 of 2)',
    second: 'Confirm document reset (2 of 2)',
    collaborativeWarning: 'This clears the summary, every section and its prose, comments, attachments, relationships and agent proposals. The document and mindmap share this content, so both views will be cleared. Title, status, metadata, existing revision history and linked tickets/checks remain. Connected editors receive the reset.',
    initiativeWarning: 'This clears the summary and all entries, views, notes, amendments, discussions, proposals and attachments. The document keeps its title, folder, metadata, status, labels and tags. Linked tickets and checks remain.',
    irreversible: 'This reset cannot be reversed with Undo. Save a copy of anything you need before continuing.',
    continue: 'Continue to final confirmation',
    typeId: 'Type the document ID to confirm',
    final: 'Clear document',
    busy: 'Clearing…',
    cancel: 'Cancel',
    success: 'Document cleared.',
  },
  de: {
    title: 'Dokument zurücksetzen',
    description: 'Ein Dokument in diesem Projekt leeren und neu beginnen. Wähle ein Dokument und bestätige zweimal.',
    search: 'Dokument suchen',
    searchHint: 'Nach Titel suchen. Nur das ausgewählte Dokument in diesem Projekt wird zurückgesetzt.',
    document: 'Dokument',
    choose: 'Dokument auswählen…',
    collaborative: 'Spezifikation',
    initiative: 'Initiative',
    loading: 'Dokumente werden geladen…',
    empty: 'Keine aktiven Dokumente für diese Suche.',
    limited: 'Weitere Dokumente passen zur Suche. Grenze die Suche ein, um das gewünschte Dokument zu finden.',
    retry: 'Erneut versuchen',
    reset: 'Dokument zurücksetzen…',
    first: 'Dieses Dokument leeren? (1 von 2)',
    second: 'Zurücksetzen bestätigen (2 von 2)',
    collaborativeWarning: 'Dies löscht Zusammenfassung, alle Abschnitte samt Text, Kommentare, Anhänge, Beziehungen und Agentenvorschläge. Dokument und Mindmap teilen diese Inhalte: Beide Ansichten werden geleert. Titel, Status, Metadaten, vorhandene Versionshistorie und verknüpfte Tickets/Checks bleiben erhalten. Verbundene Editoren erhalten die Änderung.',
    initiativeWarning: 'Dies löscht die Zusammenfassung und alle Einträge, Ansichten, Notizen, Änderungsanträge, Diskussionen, Vorschläge und Anhänge. Titel, Ordner, Metadaten, Status, Labels und Tags bleiben erhalten. Verknüpfte Tickets und Checks bleiben bestehen.',
    irreversible: 'Dieser Reset kann nicht über Rückgängig aufgehoben werden. Sichere benötigte Inhalte, bevor du fortfährst.',
    continue: 'Weiter zur letzten Bestätigung',
    typeId: 'Zur Bestätigung die Dokument-ID eingeben',
    final: 'Dokument leeren',
    busy: 'Wird geleert…',
    cancel: 'Abbrechen',
    success: 'Dokument geleert.',
  },
})

type ResetTarget = { id: string; title: string; kind: 'mindmaps' | 'initiatives' }

/** Mounted with a project/token key so navigation discards every confirmation. */
export function DocumentReset({ token, project, lang }: { token: string; project: string; lang: Locale }) {
  const t = pick(STR, lang)
  const [query, setQuery] = useState('')
  const [items, setItems] = useState<ResetTarget[]>([])
  const [selected, setSelected] = useState('')
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState('')
  const [limited, setLimited] = useState(false)
  const [reload, setReload] = useState(0)
  const [target, setTarget] = useState<ResetTarget | null>(null)
  const [step, setStep] = useState<1 | 2>(1)
  const [confirmId, setConfirmId] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState(false)

  useEffect(() => {
    let current = true
    setLoading(true)
    setLoadError('')
    setSelected('')
    const timer = setTimeout(() => {
      Promise.all([
        listMindmaps(token, { project, q: query, limit: 200 }),
        listInitiatives(token, { project, q: query, limit: 200 }),
      ]).then(([docs, initiatives]) => {
        if (!current) return
        setItems([
          ...docs.items.map((d) => ({ id: d.id, title: d.title, kind: 'mindmaps' as const })),
          ...initiatives.items.map((d) => ({ id: d.id, title: d.title, kind: 'initiatives' as const })),
        ])
        setLimited(docs.total > docs.items.length || initiatives.next_cursor !== null)
      }).catch((e: unknown) => {
        if (current) setLoadError(e instanceof Error ? e.message : String(e))
      }).finally(() => {
        if (current) setLoading(false)
      })
    }, query ? 250 : 0)
    return () => { current = false; clearTimeout(timer) }
  }, [token, project, query, reload])

  const chosen = items.find((d) => `${d.kind}/${d.id}` === selected)
  function close() {
    if (busy) return
    setTarget(null)
    setConfirmId('')
    setError('')
    setStep(1)
  }
  async function reset() {
    if (!target || step !== 2 || confirmId !== target.id || busy) return
    setBusy(true)
    setError('')
    try {
      await api(token, `/${target.kind}/${encodeURIComponent(target.id)}/reset`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ confirm_id: target.id }),
      })
      setTarget(null)
      setConfirmId('')
      setStep(1)
      setSuccess(true)
      setReload((value) => value + 1)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="border-destructive/30 flex min-w-0 flex-col gap-3 rounded-xl border p-4" aria-label={t.title}>
      <div>
        <h2 className="text-[14px] font-semibold">{t.title}</h2>
        <p className="text-muted-foreground mt-1 text-[13px]">{t.description}</p>
      </div>
      <Field label={t.search} hint={t.searchHint}>
        {(id) => <Input id={id} value={query} onChange={(e) => { setQuery(e.target.value); setSuccess(false) }} />}
      </Field>
      {loading ? <p role="status" className="text-muted-foreground text-[13px]">{t.loading}</p> : loadError ? (
        <div><p role="alert" className="text-destructive text-[13px]">{loadError}</p><Button variant="secondary" size="sm" onClick={() => setReload((value) => value + 1)}>{t.retry}</Button></div>
      ) : (
        <>
          <Field label={t.document}>
            {(id) => (
              <select id={id} className="bg-card border-input w-full min-w-0 rounded-md border px-3 py-2 text-[13px]" value={selected} onChange={(e) => { setSelected(e.target.value); setSuccess(false) }}>
                <option value="">{t.choose}</option>
                {items.map((d) => <option key={`${d.kind}/${d.id}`} value={`${d.kind}/${d.id}`}>{d.title} · {d.kind === 'mindmaps' ? t.collaborative : t.initiative} · {d.id}</option>)}
              </select>
            )}
          </Field>
          {!items.length && <p className="text-muted-foreground text-[13px]">{t.empty}</p>}
          {limited && <p className="text-muted-foreground text-[13px]">{t.limited}</p>}
        </>
      )}
      <div><Button variant="destructive" size="sm" disabled={loading || !!loadError || !chosen} onClick={() => { if (chosen) { setTarget(chosen); setStep(1); setConfirmId(''); setError(''); setSuccess(false) } }}>{t.reset}</Button></div>
      {success && <p role="status" className="text-ok text-[13px]">{t.success}</p>}
      <Dialog open={target !== null} onOpenChange={(open) => { if (!open) close() }}>
        <DialogContent className="max-w-[calc(100%-2rem)] sm:max-w-116">
          <DialogHeader>
            <DialogTitle>{step === 1 ? t.first : t.second}</DialogTitle>
            <DialogDescription asChild>
              <div className="space-y-3">
                <p className="text-foreground break-words font-semibold">{target?.title}</p>
                <p className="break-all font-mono text-xs">{project} / {target?.id}</p>
                <p>{target?.kind === 'mindmaps' ? t.collaborativeWarning : t.initiativeWarning}</p>
                <p>{t.irreversible}</p>
              </div>
            </DialogDescription>
          </DialogHeader>
          {step === 2 && <Field label={t.typeId}>{(id) => <Input id={id} autoComplete="off" value={confirmId} onChange={(e) => setConfirmId(e.target.value)} disabled={busy} />}</Field>}
          {error && <p role="alert" className="text-destructive text-[13px]">{error}</p>}
          <DialogFooter>
            <Button variant="ghost" onClick={close} disabled={busy}>{t.cancel}</Button>
            {step === 1 ? <Button variant="destructive" onClick={() => setStep(2)}>{t.continue}</Button> : <Button variant="destructive" disabled={busy || confirmId !== target?.id} onClick={() => void reset()}>{busy ? t.busy : t.final}</Button>}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
