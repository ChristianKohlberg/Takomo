import { useCallback, useState, useSyncExternalStore } from 'react'
import type * as Y from 'yjs'
import { AttachmentsDialog, type AttachmentsDialogLabels } from '@/components/mindmap/AttachmentsDialog'
import { Button } from '@/components/ui/button'
import { addAttachment, nodesMap, readAttachments, removeAttachment, updateAttachment } from '@/lib/mindmap-crdt'
import type { Attachment } from '@/lib/mindmap-doc'
import type { Locale } from '@/lib/i18n'

const LABELS: Record<Locale, AttachmentsDialogLabels> = {
  en: { title: 'References — {title}', subtitle: 'Links, repository paths and sources for this section. Shared with the map view.', empty: 'No references yet.', count: '{n} of {max} references', full: 'Maximum of {max} references reached.', kind: 'Kind', name: 'Label', gist: 'Description', ref: 'URL or repository path', add: 'Add reference', addOpen: 'Add reference', edit: 'Edit', save: 'Save', remove: 'Remove', cancel: 'Cancel', close: 'Close', readOnly: 'Read only', kinds: { pdf: 'PDF', code: 'Code', table: 'Table', diagram: 'Diagram', audio: 'Audio', link: 'Link' } },
  de: { title: 'Referenzen — {title}', subtitle: 'Links, Repository-Pfade und Quellen zu diesem Abschnitt. Gemeinsam mit der Kartenansicht.', empty: 'Noch keine Referenzen.', count: '{n} von {max} Referenzen', full: 'Höchstens {max} Referenzen möglich.', kind: 'Art', name: 'Bezeichnung', gist: 'Beschreibung', ref: 'URL oder Repository-Pfad', add: 'Referenz hinzufügen', addOpen: 'Referenz hinzufügen', edit: 'Bearbeiten', save: 'Speichern', remove: 'Entfernen', cancel: 'Abbrechen', close: 'Schließen', readOnly: 'Nur Lesen', kinds: { pdf: 'PDF', code: 'Code', table: 'Tabelle', diagram: 'Diagramm', audio: 'Audio', link: 'Link' } },
}

/** Only explicit web URLs navigate. Repository paths and other schemes stay text. */
export function referenceHref(ref: string): string | null {
  if (!/^https?:\/\//i.test(ref.trim())) return null
  try {
    const url = new URL(ref.trim())
    return url.username || url.password ? null : url.href
  } catch { return null }
}

export interface SectionReferencesProps {
  ydoc: Y.Doc
  sectionId: string
  title: string
  locale: Locale
  canWrite: boolean
  /** Wrap changes in the document's history and settled-edit trace. */
  onChange?: (change: () => void) => void
}

export function SectionReferences({ ydoc, sectionId, title, locale, canWrite, onChange }: SectionReferencesProps) {
  const [managing, setManaging] = useState(false)
  const subscribe = useCallback((notify: () => void) => {
    const nodes = nodesMap(ydoc)
    let node = nodes.get(sectionId)
    node?.observeDeep(notify)
    const changed = () => {
      const next = nodes.get(sectionId)
      if (next !== node) { node?.unobserveDeep(notify); node = next; node?.observeDeep(notify) }
      notify()
    }
    nodes.observe(changed)
    return () => { nodes.unobserve(changed); node?.unobserveDeep(notify) }
  }, [ydoc, sectionId])
  const snapshot = useCallback(() => {
    const node = nodesMap(ydoc).get(sectionId)
    return JSON.stringify(node ? readAttachments(node) : [])
  }, [ydoc, sectionId])
  const serialized = useSyncExternalStore(subscribe, snapshot, snapshot)
  const items = JSON.parse(serialized) as Attachment[]
  const labels = LABELS[locale]
  const mutate = (change: () => void) => {
    if (!canWrite || !nodesMap(ydoc).has(sectionId)) return
    if (onChange) onChange(change)
    else change()
  }
  if (!nodesMap(ydoc).has(sectionId)) return null
  return <details className="border-border-soft text-muted-foreground mt-3 min-w-0 rounded-md border px-3 py-2 text-sm">
    <summary className="cursor-pointer select-none">{locale === 'de' ? 'Referenzen' : 'References'} ({items.length})</summary>
    <div className="mt-2 min-w-0 space-y-2">
      {!items.length && <p>{labels.empty}</p>}
      {items.map(item => {
        const href = referenceHref(item.ref)
        return <div key={item.id} className="min-w-0 break-words [overflow-wrap:anywhere]">
          {href ? <a className="text-foreground underline" href={href} target="_blank" rel="noopener noreferrer">{item.name || item.ref}</a> : <span className="text-foreground font-medium">{item.name || item.ref}</span>}
          {item.ref && <div className="font-mono text-xs">{item.ref}</div>}
          {item.gist && <p className="whitespace-pre-wrap">{item.gist}</p>}
        </div>
      })}
      {canWrite && <Button size="sm" variant="outline" onClick={() => setManaging(true)}>{locale === 'de' ? 'Referenzen verwalten' : 'Manage references'}</Button>}
    </div>
    {managing && canWrite && <AttachmentsDialog node={{ id: sectionId, title, attachments: items }} canWrite={canWrite} labels={labels} onOpenChange={setManaging}
      onAdd={(id, draft) => mutate(() => { addAttachment(ydoc, id, draft) })}
      onUpdate={(id, attachment, draft) => mutate(() => { updateAttachment(ydoc, id, attachment, draft) })}
      onRemove={(id, attachment) => mutate(() => { removeAttachment(ydoc, id, attachment) })} />}
  </details>
}
