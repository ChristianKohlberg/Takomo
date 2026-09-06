import { useEffect, useLayoutEffect, useRef, useState, type MutableRefObject } from 'react'
import { createPortal } from 'react-dom'
import type { Editor } from '@tiptap/react'
import { closeSlashMenu, insertSlashBlock, type InsertKind, type SlashMatch } from '@/lib/slash-insert'
import type { Locale } from '@/lib/i18n'
import { STR } from './strings'

// A heading also has a keyboard shortcut (StarterKit's Mod-Alt-n), shown so the
// menu teaches the faster path rather than being the only one.
const choices: { kind: InsertKind; label: keyof typeof STR.en; search: string; icon: string; shortcut?: 1 | 2 | 3 }[] = [
  { kind: 'heading1', label: 'slashHeading1', search: 'heading title überschrift h1', icon: 'H1', shortcut: 1 },
  { kind: 'heading2', label: 'slashHeading2', search: 'heading subtitle überschrift h2', icon: 'H2', shortcut: 2 },
  { kind: 'heading3', label: 'slashHeading3', search: 'heading überschrift h3', icon: 'H3', shortcut: 3 },
  { kind: 'bulletList', label: 'slashBullets', search: 'bullet list liste aufzählung', icon: '•' },
  { kind: 'orderedList', label: 'slashNumbered', search: 'numbered ordered list liste nummeriert', icon: '1.' },
  { kind: 'quote', label: 'slashQuote', search: 'quote quotation blockquote zitat', icon: '❝' },
  { kind: 'code', label: 'slashCode', search: 'code block', icon: '<>' },
  { kind: 'table', label: 'tableInsert', search: 'table tabelle rows columns zeilen spalten', icon: '▦' },
  { kind: 'wireframe', label: 'slashWireframe', search: 'wireframe salt layout ui sketch skizze', icon: '▣' },
  { kind: 'plantuml', label: 'slashPlantuml', search: 'plantuml puml diagram diagramm sequence', icon: '◇' },
  { kind: 'd2', label: 'slashD2', search: 'd2 diagram diagramm architecture', icon: '◇' },
  { kind: 'mermaid', label: 'slashMermaid', search: 'mermaid diagram diagramm flowchart', icon: '◇' },
]

export function SlashMenu({ editor, match, locale, menuId, keys }: {
  editor: Editor; match: SlashMatch; locale: Locale; menuId: string
  keys: MutableRefObject<((event: KeyboardEvent) => boolean) | null>
}) {
  const t = STR[locale]
  const [selected, setSelected] = useState(0)
  const [table, setTable] = useState(false)
  const [rows, setRows] = useState('3')
  const [cols, setCols] = useState('3')
  const [position, setPosition] = useState({ left: 8, top: 8 })
  const panel = useRef<HTMLDivElement>(null)
  const results = choices.filter(c => `${t[c.label]} ${c.search}`.toLocaleLowerCase().includes(match.query.toLocaleLowerCase().trim()))
  const active = Math.min(selected, Math.max(0, results.length - 1))
  const validSize = [rows, cols].every(n => /^\d+$/.test(n) && Number(n) >= 1 && Number(n) <= 10)
  const modifier = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘⌥' : 'Ctrl+Alt+'
  const close = () => { closeSlashMenu(editor); editor.commands.focus() }
  const choose = (kind: InsertKind) => {
    if (kind === 'table') setTable(true)
    else if (!insertSlashBlock(editor, match, kind)) close()
  }
  const back = () => { setTable(false); editor.commands.focus() }

  useLayoutEffect(() => {
    const place = () => {
      if (editor.isDestroyed) return
      const coords = editor.view.coordsAtPos(Math.min(match.from, editor.state.doc.content.size))
      setPosition({ left: Math.max(8, Math.min(coords.left, window.innerWidth - 304)), top: Math.max(8, Math.min(coords.bottom + 6, window.innerHeight - 380)) })
    }
    place()
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => { window.removeEventListener('resize', place); window.removeEventListener('scroll', place, true) }
  }, [editor, match.from])

  useEffect(() => {
    const listener = (event: PointerEvent) => {
      if (event.target instanceof Node && !panel.current?.contains(event.target)) closeSlashMenu(editor)
    }
    document.addEventListener('pointerdown', listener)
    return () => document.removeEventListener('pointerdown', listener)
  }, [editor])

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { close(); return true }
      if (table) return false
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        setSelected((active + (event.key === 'ArrowDown' ? 1 : -1) + results.length) % Math.max(1, results.length)); return true
      }
      if (event.key === 'Enter') { if (results[active]) choose(results[active].kind); return true }
      if (event.key === 'Tab') { closeSlashMenu(editor); return false }
      return false
    }
    keys.current = key
    if (!table && results[active]) editor.view.dom.setAttribute('aria-activedescendant', `${menuId}-${results[active].kind}`)
    else editor.view.dom.removeAttribute('aria-activedescendant')
    return () => { keys.current = null; editor.view.dom.removeAttribute('aria-activedescendant') }
  })

  useEffect(() => { panel.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }) }, [active])

  return createPortal(<div ref={panel} style={{ position: 'fixed', ...position }}
    className="bg-popover text-popover-foreground border-border z-50 max-h-[calc(100dvh-1rem)] w-72 max-w-[calc(100vw-1rem)] overflow-y-auto rounded-lg border p-2 shadow-lg"
    onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (table) back(); else close() } }}>
    <div className="text-muted-foreground px-2 py-1 text-xs">{table ? t.slashTableSize : t.slashTitle}</div>
    {table ? <form id={menuId} role="dialog" aria-label={t.slashTableSize} onSubmit={e => { e.preventDefault(); if (validSize && !insertSlashBlock(editor, match, 'table', Number(rows), Number(cols))) close() }} className="space-y-3 p-2">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-sm">{t.slashRows}<input autoFocus type="number" min="1" max="10" value={rows} onChange={e => setRows(e.target.value)} className="border-input bg-background mt-1 w-full rounded border px-2 py-1" /></label>
        <label className="text-sm">{t.slashColumns}<input type="number" min="1" max="10" value={cols} onChange={e => setCols(e.target.value)} className="border-input bg-background mt-1 w-full rounded border px-2 py-1" /></label>
      </div>
      <p className="text-muted-foreground text-xs">{t.slashSizeHint}</p>
      <div className="flex justify-end gap-2"><button type="button" onClick={back} className="rounded px-2 py-1 text-sm hover:bg-accent">{t.slashBack}</button><button type="submit" disabled={!validSize} className="bg-primary text-primary-foreground rounded px-3 py-1 text-sm disabled:opacity-40">{t.tableInsert}</button></div>
    </form> : <>
      <div id={menuId} role="listbox" aria-label={t.slashTitle} className="max-h-64 overflow-y-auto">
        {results.map((choice, index) => <button key={choice.kind} id={`${menuId}-${choice.kind}`} role="option" aria-selected={index === active} type="button" tabIndex={-1}
          onMouseDown={e => e.preventDefault()} onPointerMove={() => setSelected(index)} onClick={() => choose(choice.kind)}
          className={`flex w-full items-center gap-3 rounded px-2 py-2 text-left text-sm ${index === active ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60'}`}>
          <span aria-hidden className="text-muted-foreground w-6 shrink-0 text-center text-xs">{choice.icon}</span>
          <span className="min-w-0 flex-1">{t[choice.label]}</span>
          {choice.shortcut && <kbd className="text-muted-foreground shrink-0 text-xs">{modifier}{choice.shortcut}</kbd>}
        </button>)}
        {!results.length && <p role="status" className="text-muted-foreground px-2 py-3 text-sm">{t.slashEmpty}</p>}
      </div>
      <p className="text-muted-foreground border-border mt-1 border-t px-2 pt-2 text-[11px]">{t.slashHint}</p>
    </>}
  </div>, document.body)
}
