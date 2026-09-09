import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type MutableRefObject } from 'react'
import { createPortal } from 'react-dom'
import type { Editor } from '@tiptap/react'
import type * as Y from 'yjs'
import type { Locale } from '@/lib/i18n'
import { sectionReferenceIndex, searchReferenceSections } from '@/lib/section-reference-index'
import { closeReferenceMenu, insertReferenceMatch, type ReferenceMatch } from '@/lib/section-reference-trigger'
export function SectionReferenceMenu({ editor, ydoc, match, locale, menuId, keys, boundary }: {
  editor: Editor; ydoc: Y.Doc; match: ReferenceMatch; locale: Locale; menuId: string
  boundary?: () => void
  keys: MutableRefObject<((event: KeyboardEvent) => boolean) | null>
}) {
  const index = sectionReferenceIndex(ydoc)
  const sections = useSyncExternalStore(index.subscribe, index.getSnapshot, index.getSnapshot)
  const results = searchReferenceSections(sections, match.query)
  const visible = results.slice(0, 50)
  const [selected, setSelected] = useState(0)
  const active = Math.min(selected, Math.max(0, visible.length - 1))
  const [position, setPosition] = useState({ left: 8, top: 8 })
  const panel = useRef<HTMLDivElement>(null)
  const de = locale === 'de'
  const choose = (id: string) => { if (!insertReferenceMatch(editor, ydoc, match, id, boundary)) closeReferenceMenu(editor) }
  useLayoutEffect(() => {
    const place = () => {
      if (editor.isDestroyed) return
      const coords = editor.view.coordsAtPos(Math.min(match.from, editor.state.doc.content.size))
      const viewport = window.visualViewport
      const left = viewport?.offsetLeft ?? 0
      const top = viewport?.offsetTop ?? 0
      const width = viewport?.width ?? window.innerWidth
      const height = viewport?.height ?? window.innerHeight
      setPosition({ left: Math.max(left + 8, Math.min(coords.left, left + width - 320)), top: Math.max(top + 8, Math.min(coords.bottom + 6, top + height - 320)) })
      if (panel.current) { panel.current.style.maxHeight = `${Math.max(80, height - 16)}px`; panel.current.style.maxWidth = `${Math.max(80, width - 16)}px` }
    }
    const viewport = window.visualViewport
    viewport?.addEventListener('resize', place); viewport?.addEventListener('scroll', place)
    place(); window.addEventListener('resize', place); window.addEventListener('scroll', place, true)
    return () => { viewport?.removeEventListener('resize', place); viewport?.removeEventListener('scroll', place); window.removeEventListener('resize', place); window.removeEventListener('scroll', place, true) }
  }, [editor, match.from])
  useEffect(() => {
    const dismiss = (event: PointerEvent) => { if (event.target instanceof Node && !panel.current?.contains(event.target)) closeReferenceMenu(editor) }
    document.addEventListener('pointerdown', dismiss)
    return () => document.removeEventListener('pointerdown', dismiss)
  }, [editor])
  useEffect(() => {
    keys.current = event => {
      if (event.key === 'Escape') { closeReferenceMenu(editor); return true }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        setSelected((active + (event.key === 'ArrowDown' ? 1 : -1) + visible.length) % Math.max(1, visible.length)); return true
      }
      if (event.key === 'Enter') { if (visible[active]) choose(visible[active].key); return true }
      if (event.key === 'Tab') closeReferenceMenu(editor)
      return false
    }
    if (visible[active]) editor.view.dom.setAttribute('aria-activedescendant', `${menuId}-${active}`)
    else editor.view.dom.removeAttribute('aria-activedescendant')
    return () => { keys.current = null; editor.view.dom.removeAttribute('aria-activedescendant') }
  })
  useEffect(() => { panel.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }) }, [active])
  return createPortal(<div ref={panel} style={{ position: 'fixed', ...position }} className="bg-popover text-popover-foreground border-border z-50 overflow-y-auto w-80 max-w-[calc(100vw-1rem)] rounded-lg border p-2 shadow-lg">
    <p className="px-2 py-1 text-xs text-muted-foreground">{de ? 'Abschnitt verknüpfen' : 'Link to section'}</p>
    <div id={menuId} role="listbox" aria-label={de ? 'Abschnitte' : 'Sections'} className="max-h-56 overflow-y-auto">
      {visible.map((section, row) => <button key={section.key} id={`${menuId}-${row}`} type="button" role="option" aria-selected={row === active} tabIndex={-1}
        onMouseDown={event => event.preventDefault()} onPointerMove={() => setSelected(row)} onClick={() => choose(section.key)}
        className={`block w-full break-words rounded px-2 py-2 text-left text-sm ${row === active ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60'}`}>
        <span className="mr-2 text-muted-foreground">{section.number}</span>{section.title || (de ? 'Unbenannter Abschnitt' : 'Untitled section')}
      </button>)}
      {!visible.length && <p role="status" className="px-2 py-3 text-sm text-muted-foreground">{de ? 'Keine Abschnitte gefunden' : 'No sections found'}</p>}
    </div>
    <p role="status" className="border-border mt-1 border-t px-2 pt-2 text-xs text-muted-foreground">{results.length > visible.length ? `${visible.length} / ${results.length} · ` : ''}{de ? '↑↓ Auswählen · Enter Einfügen · Esc Schließen' : '↑↓ Choose · Enter Insert · Esc Dismiss'}</p>
  </div>, document.body)
}
