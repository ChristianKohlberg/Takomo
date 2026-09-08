import { Search, Undo2, Redo2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { useState, type ReactNode } from 'react'

export function DocumentActions({ focusMode = false, locale, findOpen, onFind, canWrite, textUndo, textRedo, moveUndo, moveRedo, onTextUndo, onTextRedo, onMoveUndo, onMoveRedo, children, primary }: {
  focusMode?: boolean
  children?: ReactNode
  primary?: ReactNode
  locale: Locale; findOpen: boolean; onFind: () => void; canWrite: boolean
  textUndo: boolean; textRedo: boolean; moveUndo: boolean; moveRedo: boolean
  onTextUndo: () => void; onTextRedo: () => void; onMoveUndo: () => void; onMoveRedo: () => void
}) {
  const de = locale === 'de'
  const [expanded, setExpanded] = useState(false)
  return <div className="document-toolbar relative flex flex-none flex-wrap items-center gap-1 border-b border-border-soft bg-card px-3 py-1.5" role="toolbar" aria-label={de ? 'Dokumentwerkzeuge' : 'Document tools'}>
    {primary}
    <button type="button" className="document-tools-toggle rounded border px-2 py-1 text-xs" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>{de ? 'Werkzeuge' : 'Tools'} {expanded ? '−' : '+'}</button>
    <div className="document-more-tools" data-open={expanded || undefined} onClick={event => { const button = (event.target as Element).closest('button'); if (button && !button.hasAttribute('aria-haspopup') && !button.closest('[data-tools-stay]')) setExpanded(false) }} onKeyDown={event => { if (event.key === 'Escape') setExpanded(false) }}>
    {children}
    {!focusMode && <Button variant="ghost" size="sm" aria-label={de ? 'Im Dokument suchen' : 'Find in document'} title={de ? 'Im Dokument suchen' : 'Find in document'} aria-expanded={findOpen} onClick={onFind}><Search className="size-3.5" aria-hidden="true" /><span className="hidden sm:inline">{de ? 'Im Dokument suchen' : 'Find in document'}</span></Button>}
    {canWrite && !focusMode && <>
      <div className="flex items-center gap-1" role="group" data-tools-stay aria-label={de ? 'Text im aktuellen Abschnitt' : 'Current section text'}>
        <span className="hidden text-xs text-muted-foreground sm:inline">{de ? 'Abschnitt' : 'Section'}</span>
        <Button variant="ghost" size="icon-sm" disabled={!textUndo} aria-label={de ? 'Text rückgängig' : 'Undo section text'} title={de ? 'Text rückgängig' : 'Undo section text'} onMouseDown={e => e.preventDefault()} onClick={onTextUndo}><Undo2 className="size-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!textRedo} aria-label={de ? 'Text wiederholen' : 'Redo section text'} title={de ? 'Text wiederholen' : 'Redo section text'} onMouseDown={e => e.preventDefault()} onClick={onTextRedo}><Redo2 className="size-4" /></Button>
      </div>
      <div className="flex items-center gap-1" role="group" data-tools-stay aria-label={de ? 'Abschnitt verschieben' : 'Section moves'}>
        <span className="hidden text-xs text-muted-foreground sm:inline">{de ? 'Verschieben' : 'Move'}</span>
        <Button variant="ghost" size="icon-sm" disabled={!moveUndo} aria-label={de ? 'Verschieben rückgängig' : 'Undo section move'} title={de ? 'Verschieben rückgängig' : 'Undo section move'} onClick={onMoveUndo}><Undo2 className="size-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!moveRedo} aria-label={de ? 'Verschieben wiederholen' : 'Redo section move'} title={de ? 'Verschieben wiederholen' : 'Redo section move'} onClick={onMoveRedo}><Redo2 className="size-4" /></Button>
      </div>
    </>}
    </div>
  </div>
}
