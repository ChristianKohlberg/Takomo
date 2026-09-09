import { Undo2, Redo2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { useState, type ReactNode } from 'react'

export function DocumentActions({ focusMode = false, locale, canWrite, canUndo, canRedo, onUndo, onRedo, children, primary }: {
  focusMode?: boolean
  children?: ReactNode
  primary?: ReactNode
  locale: Locale; canWrite: boolean
  canUndo: boolean; canRedo: boolean
  onUndo: () => void; onRedo: () => void
}) {
  const de = locale === 'de'
  const [expanded, setExpanded] = useState(false)
  return <div className="document-toolbar relative flex flex-none flex-wrap items-center gap-1 border-b border-border-soft bg-card px-3 py-1.5" role="toolbar" aria-label={de ? 'Dokumentwerkzeuge' : 'Document tools'}>
    {primary}
    <button type="button" className="document-tools-toggle rounded border px-2 py-1 text-xs" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>{de ? 'Werkzeuge' : 'Tools'} {expanded ? '−' : '+'}</button>
    <div className="document-more-tools" data-open={expanded || undefined} onClick={event => { const button = (event.target as Element).closest('button'); if (button && !button.hasAttribute('aria-haspopup') && !button.closest('[data-tools-stay]')) setExpanded(false) }} onKeyDown={event => { if (event.key === 'Escape') setExpanded(false) }}>
      {children}
      {canWrite && !focusMode && <div className="flex items-center gap-1" role="group" data-tools-stay aria-label={de ? 'Dokumentverlauf' : 'Document history'}>
        <Button variant="ghost" size="icon-sm" disabled={!canUndo} aria-label={de ? 'Rückgängig' : 'Undo'} title={de ? 'Rückgängig' : 'Undo'} onMouseDown={event => event.preventDefault()} onClick={onUndo}><Undo2 className="size-4" aria-hidden="true" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!canRedo} aria-label={de ? 'Wiederholen' : 'Redo'} title={de ? 'Wiederholen' : 'Redo'} onMouseDown={event => event.preventDefault()} onClick={onRedo}><Redo2 className="size-4" aria-hidden="true" /></Button>
      </div>}
    </div>
  </div>
}
