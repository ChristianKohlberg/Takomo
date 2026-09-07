import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'
import type { Editor } from '@tiptap/react'
import type * as Y from 'yjs'
import { Link2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { flattenSections, planSections } from '@/lib/plan-sections'
import { nodesMap, readPlanTree } from '@/lib/mindmap-crdt'
import type { Locale } from '@/lib/i18n'

/** Opening the picker keeps the editor's selection; incoming edits map it as usual. */
export function DocumentSectionReferenceButton({ editor, ydoc, canWrite, locale }: {
  editor: Editor | null; ydoc: Y.Doc; canWrite: boolean; locale: Locale
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [, refresh] = useState(0)
  const de = locale === 'de'
  const subscribe = useCallback((notify: () => void) => {
    editor?.on('selectionUpdate', notify).on('transaction', notify).on('destroy', notify)
    return () => { editor?.off('selectionUpdate', notify).off('transaction', notify).off('destroy', notify) }
  }, [editor])
  const snapshot = useCallback(() => !!editor && !editor.isDestroyed && editor.isEditable && !!editor.schema.nodes.sectionReference &&
    editor.can().insertContent({ type: 'sectionReference', attrs: { sectionId: 'preview' } }), [editor])
  const available = useSyncExternalStore(subscribe, snapshot, () => false)
  useEffect(() => {
    if (!open) return
    const nodes = nodesMap(ydoc)
    const update = () => refresh(value => value + 1)
    nodes.observeDeep(update)
    return () => nodes.unobserveDeep(update)
  }, [open, ydoc])
  if (!canWrite) return null
  const results = open ? flattenSections(planSections(readPlanTree(ydoc))).filter(node => node.title.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) : []
  return <Popover open={open && available} onOpenChange={value => { setQuery(''); setOpen(value) }}>
    <PopoverTrigger asChild><Button variant="ghost" size="icon-sm" disabled={!available} aria-label={de ? 'Abschnitt verknüpfen' : 'Insert section reference'}
      title={de ? 'Abschnitt verknüpfen' : 'Insert section reference'} onMouseDown={event => event.preventDefault()}>
      <Link2 className="size-3.5" aria-hidden="true" />
    </Button></PopoverTrigger>
    <PopoverContent aria-label={de ? 'Abschnitt verknüpfen' : 'Insert section reference'} collisionPadding={12}
      className="w-72 max-w-[calc(100vw-1.5rem)]" onCloseAutoFocus={event => { event.preventDefault(); if (editor && !editor.isDestroyed) editor.commands.focus() }}>
      <input type="search" aria-label={de ? 'Abschnitte suchen' : 'Search sections'} value={query} onChange={event => setQuery(event.target.value)}
        className="w-full rounded border bg-background px-2 py-1 text-sm" />
      <div className="max-h-60 overflow-y-auto">
        {results.map(node => <button key={node.key} type="button" className="block w-full break-words rounded px-2 py-1 text-left text-sm hover:bg-muted focus-visible:bg-muted"
          onClick={() => {
            if (!editor || editor.isDestroyed || !editor.isEditable || !nodesMap(ydoc).has(node.key)) return
            editor.chain().focus().insertContent({ type: 'sectionReference', attrs: { sectionId: node.key }, content: node.title ? [{ type: 'text', text: node.title }] : [] }).run()
            setOpen(false)
          }}><span className="mr-1 text-muted-foreground">{node.number}</span>{node.title || (de ? 'Unbenannter Abschnitt' : 'Untitled section')}</button>)}
        {!results.length && <p className="p-2 text-xs text-muted-foreground">{de ? 'Keine Abschnitte gefunden' : 'No sections found'}</p>}
      </div>
      <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>{de ? 'Abbrechen' : 'Cancel'}</Button>
    </PopoverContent>
  </Popover>
}
