import { useState } from 'react'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { flattenSections, type PlanSection } from '@/lib/plan-sections'
import type { SectionPlacement, StructureResult } from '@/lib/plan-structure'

export interface MoveSectionDialogProps {
  sections: PlanSection[]
  sectionKey: string
  lang: 'en' | 'de'
  onClose: () => void
  onMove: (id: string, target: string, placement: SectionPlacement) => StructureResult
}

export function MoveSectionDialog({ sections, sectionKey, lang, onClose, onMove }: MoveSectionDialogProps) {
  const de = lang === 'de'
  const rows = flattenSections(sections)
  const source = rows.find(s => s.key === sectionKey)
  const descendants = source ? new Set(flattenSections([source]).map(s => s.key)) : new Set([sectionKey])
  const choices = rows.filter(s => !descendants.has(s.key))
  const [target, setTarget] = useState('')
  const [placement, setPlacement] = useState<SectionPlacement>('after')
  const [error, setError] = useState('')
  const destination = rows.find(s => s.key === target)
  const depth = destination ? destination.depth + (placement === 'child' ? 1 : 0) : 0
  const label = (s: PlanSection) => `${s.number} ${s.title || (de ? 'Ohne Titel' : 'Untitled')}`
  const relation = placement === 'before' ? (de ? 'Vor' : 'Before') : placement === 'after' ? (de ? 'Nach' : 'After') : (de ? 'Als Unterabschnitt von' : 'As a child of')
  return <Dialog open onOpenChange={open => { if (!open) onClose() }}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>{de ? 'Abschnitt verschieben' : 'Move section'}</DialogTitle>
        <DialogDescription>{source ? label(source) : (de ? 'Abschnitt nicht mehr verfügbar' : 'Section no longer available')}</DialogDescription>
      </DialogHeader>
      <label className="grid gap-1 text-sm">{de ? 'Position' : 'Position'}
        <select className="rounded border bg-background p-2" value={placement} onChange={event => setPlacement(event.target.value as SectionPlacement)}>
          <option value="before">{de ? 'Davor' : 'Before'}</option>
          <option value="after">{de ? 'Danach' : 'After'}</option>
          <option value="child">{de ? 'Als Unterabschnitt' : 'As a child'}</option>
        </select>
      </label>
      <label className="grid gap-1 text-sm">{de ? 'Zielabschnitt' : 'Destination section'}
        <select className="min-w-0 rounded border bg-background p-2" value={target} onChange={event => { setTarget(event.target.value); setError('') }}>
          <option value="">{de ? 'Abschnitt auswählen' : 'Select a section'}</option>
          {choices.map(s => <option key={s.key} value={s.key}>{label(s)}</option>)}
        </select>
      </label>
      {destination && <p className="rounded bg-muted p-3 text-sm" aria-live="polite">{relation} <strong>{label(destination)}</strong>. {de ? 'Neue Ebene' : 'New level'}: {depth + 1}. {de ? 'Text, Unterabschnitte und Verknüpfungen bleiben erhalten.' : 'Text, children and links stay attached.'}</p>}
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={onClose}>{de ? 'Abbrechen' : 'Cancel'}</Button>
        <Button disabled={!source || !target} onClick={() => {
          const result = onMove(sectionKey, target, placement)
          if (result.ok) onClose()
          else setError(result.error === 'cycle'
            ? (de ? 'Ein Abschnitt kann nicht in sich selbst verschoben werden.' : 'A section cannot move into itself or its children.')
            : (de ? 'Das Ziel hat sich geändert oder wurde entfernt. Bitte erneut auswählen. Der Inhalt bleibt erhalten.' : 'The destination changed or was removed. Select it again. Your content is preserved.'))
        }}>{de ? 'Verschieben' : 'Move'}</Button>
      </div>
    </DialogContent>
  </Dialog>
}
