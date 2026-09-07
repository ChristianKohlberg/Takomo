import { useEffect, useRef, useState } from 'react'
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
  selected: string | null; canAsk: boolean; onError: (error: unknown) => void
}
/** Only mounted in the document view; the map keeps its existing command menu. */
export function DocumentAgent(props: Props) {
  return <Agent key={`${props.token}:${props.map}`} {...props} />
}
function Agent({ selected, ...props }: Props) {
  const t = DOCUMENT_CHAT[props.lang]
  const [open, setOpen] = useState(false)
  const [menu, setMenu] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const [intent, setIntent] = useState<DocumentIntent>({ action: 'discuss', section_ids: [], whole_document: true, nonce: 0 })
  const returnFocus = useRef<HTMLElement | null>(null)
  const movingToSheet = useRef(false)
  const restoreFocus = () => { movingToSheet.current = false; if (returnFocus.current?.isConnected) returnFocus.current.focus({ preventScroll: true }) }
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
  const actions: DocumentAction[] = props.canAsk ? ['discuss', 'grill', 'draft_tests', 'draft_questions'] : ['discuss']
  const filtered = actions.filter(action => `${action === 'discuss' ? t.open : t[action]} Codex`.toLocaleLowerCase().includes(query.toLocaleLowerCase()))
  function choose(action: DocumentAction) {
    const section = selected && props.nodes.some(node => node.id === selected) ? selected : null
    if (action !== 'discuss') setIntent(previous => ({ action, nonce: previous.nonce + 1, whole_document: false, section_ids: section ? [section] : [] }))
    movingToSheet.current = true
    setMenu(false); setOpen(true)
  }
  return <>
    <Button variant="ghost" size="sm" aria-label={t.open} title={`${t.open} (Ctrl/⌘ K)`} onClick={() => { returnFocus.current = document.activeElement as HTMLElement; setOpen(true) }}>
      <MessageCircle className="size-4" aria-hidden="true" /><span>Codex</span><kbd className="hidden text-xs text-muted-foreground sm:inline">⌘K</kbd>
    </Button>
    <Dialog open={menu} onOpenChange={setMenu}>
      <DialogContent onCloseAutoFocus={event => { event.preventDefault(); if (!movingToSheet.current) restoreFocus() }} className="gap-3">
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
    <DocumentConversation {...props} open={open} onOpenChange={setOpen} restoreFocus={restoreFocus} intent={intent} />
  </>
}
