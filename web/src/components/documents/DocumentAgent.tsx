import { useEffect, useRef, useState, type ReactNode, type CSSProperties } from 'react'
import { MessageCircle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { Locale } from '@/lib/i18n'
import type { PlanNode } from '@/lib/plan-sections'
import type { DocumentAction } from '@/lib/document-conversation'
import { DocumentConversation, type DocumentIntent } from './DocumentConversation'
import { DOCUMENT_CHAT } from './document-conversation-strings'

interface Props {
  token: string; project: string; map: string; lang: Locale; nodes: PlanNode[]
  children?: (tools: ReactNode) => ReactNode; onNavigate?: (section: string) => void
  selected: string | null; canAsk: boolean; onError: (error: unknown) => void
}
/** Only mounted in the document view; the map keeps its existing command menu. */
export function DocumentAgent(props: Props) {
  return <Agent key={`${props.token}:${props.map}`} {...props} />
}
function Agent({ selected, children, onNavigate, ...props }: Props) {
  const t = DOCUMENT_CHAT[props.lang]
  const [open, setOpen] = useState(false)
  const [mobile, setMobile] = useState<'document' | 'chat'>('document')
  const [width, setWidth] = useState(420)
  const workspace = useRef<HTMLDivElement>(null)
  const [wide, setWide] = useState(true)
  useEffect(() => {
    if (!workspace.current || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      const entry = entries[0]
      if (entry) setWide(entry.contentRect.width >= 720)
    })
    observer.observe(workspace.current)
    return () => observer.disconnect()
  }, [])
  const [quote, setQuote] = useState<{ section_id: string; text: string } | null>(null)
  const dragCleanup = useRef<(() => void) | null>(null)
  useEffect(() => () => dragCleanup.current?.(), [])
  const [menu, setMenu] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [intent, setIntent] = useState<DocumentIntent>({ action: 'discuss', section_ids: [], whole_document: false, mode: 'automatic', nonce: 0 })
  const returnFocus = useRef<HTMLElement | null>(null)
  const movingToPanel = useRef(false)
  const restoreFocus = () => { movingToPanel.current = false; requestAnimationFrame(() => { if (returnFocus.current?.isConnected) returnFocus.current.focus({ preventScroll: true }) }) }
  useEffect(() => {
    function key(event: KeyboardEvent) {
      if (event.key.toLowerCase() !== 'k' || !(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey || event.repeat || event.isComposing) return
      if (!menu && document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"], dialog[open]')) return
      event.preventDefault(); event.stopPropagation()
      if (!menu) { returnFocus.current = document.activeElement as HTMLElement; setQuery(''); setActive(0) }
      setMenu(value => !value)
    }
    window.addEventListener('keydown', key, true)
    return () => window.removeEventListener('keydown', key, true)
  }, [menu])
  useEffect(() => {
    function selection() {
      const selection = window.getSelection()
      const text = selection?.toString().trim() ?? ''
      const element = (node: Node | null | undefined) => node instanceof Element ? node : node?.parentElement
      const start = element(selection?.anchorNode)?.closest<HTMLElement>('[data-section]')
      const end = element(selection?.focusNode)?.closest<HTMLElement>('[data-section]')
      setQuote(text && new TextEncoder().encode(text).length <= 12000 && start === end && start && workspace.current?.contains(start) ? { section_id: start.dataset.section!, text } : null)
    }
    document.addEventListener('selectionchange', selection)
    return () => document.removeEventListener('selectionchange', selection)
  }, [])
  function navigate(section: string) {
    setMobile('document'); onNavigate?.(section)
    requestAnimationFrame(() => {
      const element = Array.from(workspace.current?.querySelectorAll<HTMLElement>('[data-section]') ?? []).find(node => node.dataset.section === section)
      element?.scrollIntoView({ block: 'center', behavior: 'smooth' })
      element?.animate?.([{ backgroundColor: 'var(--color-accent)' }, { backgroundColor: 'transparent' }], { duration: 1800 })
    })
  }
  function show() { setOpen(true); setMobile('chat') }
  const actions: DocumentAction[] = props.canAsk ? ['discuss', 'grill', 'draft_tests', 'draft_questions'] : ['discuss']
  const filtered = actions.filter(action => `${action === 'discuss' ? t.open : t[action]} Codex`.toLocaleLowerCase().includes(query.toLocaleLowerCase()))
  function choose(action: DocumentAction) {
    const section = selected && props.nodes.some(node => node.id === selected) ? selected : null
    if (action !== 'discuss') setIntent(previous => ({ action, nonce: previous.nonce + 1, whole_document: false, mode: section ? 'selected' : 'automatic', section_ids: section ? [section] : [] }))
    movingToPanel.current = true
    setMenu(false); show()
  }
  const toolbar = <>
    <Button variant="ghost" size="sm" aria-label={t.open} title={`${t.open} (Ctrl/⌘ K)`} onClick={() => { returnFocus.current = document.activeElement as HTMLElement; show() }}>
      <MessageCircle className="size-4" aria-hidden="true" /><span>{t.open}</span><kbd className="hidden text-xs text-muted-foreground sm:inline">⌘K</kbd>
    </Button>
    {quote && props.canAsk && <Button size="sm" variant="outline" onMouseDown={event => event.preventDefault()} onClick={() => {
      setIntent(previous => ({ ...previous, action: 'discuss', mode: 'selected', section_ids: [quote.section_id], whole_document: false, quote, nonce: previous.nonce + 1 })); show()
    }}>{t.discussQuote}</Button>}
  </>
  return <div ref={workspace} className="@container/document-workspace flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
    {open && <div role="tablist" aria-label={t.workspace} className="flex shrink-0 border-b @min-[720px]/document-workspace:hidden">
      {(['document', 'chat'] as const).map(tab => <button type="button" role="tab" aria-selected={mobile === tab} tabIndex={mobile === tab ? 0 : -1} onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setMobile(tab === 'document' ? 'chat' : 'document'); (event.currentTarget.parentElement?.querySelectorAll('button')[tab === 'document' ? 1 : 0] as HTMLElement)?.focus() } }} key={tab} className="flex-1 px-3 py-2 text-sm aria-selected:bg-muted" onClick={() => setMobile(tab)}>{t[tab]}</button>)}
    </div>}
    <div className="flex min-h-0 min-w-0 flex-1" style={{ '--chat-width': `${width}px` } as CSSProperties}>
      <div className={`${open && mobile === 'chat' ? 'hidden @min-[720px]/document-workspace:flex' : 'flex'} min-h-0 min-w-0 flex-1 flex-col`}>{children ? children(toolbar) : toolbar}</div>
      {open && <div role="separator" aria-label={t.resize} aria-orientation="vertical" aria-valuemin={300} aria-valuemax={720} aria-valuenow={width} tabIndex={0} className="hidden w-1 shrink-0 cursor-col-resize bg-border-soft hover:bg-primary @min-[720px]/document-workspace:block"
        onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setWidth(value => Math.max(300, Math.min(720, value + (event.key === 'ArrowLeft' ? 24 : -24)))) } }}
        onPointerDown={event => {
          event.preventDefault(); const start = event.clientX; const initial = width
          const move = (next: PointerEvent) => setWidth(Math.max(300, Math.min(720, initial + start - next.clientX)))
          const stop = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', stop); window.removeEventListener('pointercancel', stop) }
          dragCleanup.current?.(); dragCleanup.current = stop
          window.addEventListener('pointermove', move); window.addEventListener('pointerup', stop); window.addEventListener('pointercancel', stop)
        }} />}
      <div className={`${!open || mobile === 'document' ? 'hidden' : 'flex'} ${open ? '@min-[720px]/document-workspace:flex' : ''} min-h-0 w-full min-w-0 flex-col @min-[720px]/document-workspace:w-[min(var(--chat-width),55%)] @min-[720px]/document-workspace:shrink-0`}>
        <DocumentConversation {...props} open={open} visible={wide || mobile === 'chat'} onOpenChange={value => { setOpen(value); if (!value) setMobile('document') }} restoreFocus={restoreFocus} intent={intent} onNavigate={navigate} />
      </div>
    </div>
    <Dialog open={menu} onOpenChange={setMenu}>
      <DialogContent onCloseAutoFocus={event => { event.preventDefault(); if (!movingToPanel.current) restoreFocus() }} className="gap-3">
        <DialogHeader><DialogTitle>{t.commands}</DialogTitle><DialogDescription>{t.description}</DialogDescription></DialogHeader>
        <input aria-label={t.search} placeholder={t.search} value={query} onChange={event => { setQuery(event.target.value); setActive(0) }}
          className="rounded border border-border-soft bg-background px-3 py-2 text-sm" role="combobox" aria-expanded="true" aria-controls="document-agent-actions" aria-activedescendant={filtered.length ? `document-action-${filtered[Math.min(active, filtered.length - 1)]}` : undefined}
          onKeyDown={event => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setActive(value => filtered.length ? (value + (event.key === 'ArrowDown' ? 1 : -1) + filtered.length) % filtered.length : 0) }
            if (event.key === 'Enter' && filtered.length) { event.preventDefault(); choose(filtered[Math.min(active, filtered.length - 1)]!) }
          }} />
        <div id="document-agent-actions" role="listbox" aria-label={t.commands} className="space-y-1">
          {!filtered.length && <p className="p-2 text-sm text-muted-foreground">{t.noMatch}</p>}
          {filtered.map((action, index) => <button key={action} id={`document-action-${action}`} type="button" role="option" aria-selected={index === active} onMouseEnter={() => setActive(index)} onClick={() => choose(action)} className={`w-full rounded px-3 py-2 text-left text-sm hover:bg-muted ${index === active ? 'bg-muted' : ''}`}>{action === 'discuss' ? t.open : t[action]}</button>)}
        </div>
      </DialogContent>
    </Dialog>
  </div>
}
