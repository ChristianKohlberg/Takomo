import { Search, Undo2, Redo2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'

export function DocumentActions({ locale, findOpen, onFind, canWrite, textUndo, textRedo, moveUndo, moveRedo, onTextUndo, onTextRedo, onMoveUndo, onMoveRedo }: {
  locale: Locale; findOpen: boolean; onFind: () => void; canWrite: boolean
  textUndo: boolean; textRedo: boolean; moveUndo: boolean; moveRedo: boolean
  onTextUndo: () => void; onTextRedo: () => void; onMoveUndo: () => void; onMoveRedo: () => void
}) {
  const de = locale === 'de'
  return <div className="flex flex-none flex-wrap items-center gap-2 border-b border-border-soft bg-card px-3 py-1.5" role="toolbar" aria-label={de ? 'Dokumentwerkzeuge' : 'Document tools'}>
    <Button variant="ghost" size="sm" aria-expanded={findOpen} onClick={onFind}><Search className="size-3.5" aria-hidden="true" />{de ? 'Im Dokument suchen' : 'Find in document'}</Button>
    {canWrite && <>
      <div className="flex items-center gap-1" role="group" aria-label={de ? 'Text im aktuellen Abschnitt' : 'Current section text'}>
        <span className="text-xs text-muted-foreground">{de ? 'Abschnitt' : 'Section'}</span>
        <Button variant="ghost" size="icon-sm" disabled={!textUndo} aria-label={de ? 'Text rückgängig' : 'Undo section text'} title={de ? 'Text rückgängig' : 'Undo section text'} onMouseDown={e => e.preventDefault()} onClick={onTextUndo}><Undo2 className="size-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!textRedo} aria-label={de ? 'Text wiederholen' : 'Redo section text'} title={de ? 'Text wiederholen' : 'Redo section text'} onMouseDown={e => e.preventDefault()} onClick={onTextRedo}><Redo2 className="size-4" /></Button>
      </div>
      <div className="flex items-center gap-1" role="group" aria-label={de ? 'Abschnitt verschieben' : 'Section moves'}>
        <span className="text-xs text-muted-foreground">{de ? 'Verschieben' : 'Move'}</span>
        <Button variant="ghost" size="icon-sm" disabled={!moveUndo} aria-label={de ? 'Verschieben rückgängig' : 'Undo section move'} title={de ? 'Verschieben rückgängig' : 'Undo section move'} onClick={onMoveUndo}><Undo2 className="size-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!moveRedo} aria-label={de ? 'Verschieben wiederholen' : 'Redo section move'} title={de ? 'Verschieben wiederholen' : 'Redo section move'} onClick={onMoveRedo}><Redo2 className="size-4" /></Button>
      </div>
    </>}
  </div>
}
