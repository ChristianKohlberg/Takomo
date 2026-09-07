import { useEffect, useRef, useState, Fragment } from 'react'
import { useEditorState, type Editor } from '@tiptap/react'
import { ChevronDown } from 'lucide-react'
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuGroup, DropdownMenuLabel, DropdownMenuItem, DropdownMenuSeparator } from '@/components/ui/dropdown-menu'

export interface TableLabels {
  tableActions: string; tableRows: string; tableColumns: string; tableCells: string
  tableRowBefore: string; tableRowAfter: string; tableRowDelete: string
  tableColumnBefore: string; tableColumnAfter: string; tableColumnDelete: string
  tableHeaderRow: string; tableHeaderColumn: string; tableMerge: string; tableSplit: string
  tableDelete: string; tableHint: string
}

/** Keep the editor's mapped selection while the shared menu owns DOM focus. */
export function TableToolbar({ editor, labels, disabled }: { editor: Editor; labels: TableLabels; disabled: boolean }) {
  useEditorState({ editor, selector: ({ transactionNumber }) => transactionNumber })
  const [active, setActive] = useState(editor.isFocused)
  const [open, setOpen] = useState(false)
  const trigger = useRef<HTMLButtonElement>(null)
  const content = useRef<HTMLDivElement>(null)
  const restoreFocus = useRef(true)
  const inside = editor.isActive('table') && editor.isEditable && !disabled
  useEffect(() => {
    const focus = (event: Event) => {
      const target = event.target as Node
      const here = editor.view.dom.contains(target) || !!trigger.current?.contains(target) || !!content.current?.contains(target)
      setActive(here)
      if (!here) { restoreFocus.current = false; setOpen(false) }
    }
    document.addEventListener('focusin', focus)
    document.addEventListener('pointerdown', focus)
    return () => { document.removeEventListener('focusin', focus); document.removeEventListener('pointerdown', focus) }
  }, [editor])
  useEffect(() => { if (!inside) setOpen(false) }, [inside])
  if (!inside || (!active && !open)) return null
  const groups = [
    ['tableRows', [['tableRowBefore', 'addRowBefore'], ['tableRowAfter', 'addRowAfter'], ['tableRowDelete', 'deleteRow']]],
    ['tableColumns', [['tableColumnBefore', 'addColumnBefore'], ['tableColumnAfter', 'addColumnAfter'], ['tableColumnDelete', 'deleteColumn']]],
    ['tableCells', [['tableHeaderRow', 'toggleHeaderRow'], ['tableHeaderColumn', 'toggleHeaderColumn'], ['tableMerge', 'mergeCells'], ['tableSplit', 'splitCell']]],
  ] as const
  return <div className="mb-2">
    <DropdownMenu modal={false} open={open} onOpenChange={value => { if (value) restoreFocus.current = true; setOpen(value) }}>
      <DropdownMenuTrigger ref={trigger} className="text-muted-foreground hover:bg-accent focus-visible:ring-ring inline-flex items-center gap-1 rounded px-2 py-1 text-xs outline-none focus-visible:ring-2">
        {labels.tableActions}<ChevronDown aria-hidden className="size-3" />
      </DropdownMenuTrigger>
      <DropdownMenuContent ref={content} aria-label={labels.tableActions} className="w-64 max-w-[calc(100vw-1rem)]"
        onInteractOutside={() => { restoreFocus.current = false }}
        onCloseAutoFocus={event => { event.preventDefault(); if (restoreFocus.current && !editor.isDestroyed && editor.isEditable) editor.commands.focus() }}>
        {groups.map(([heading, actions], index) => <Fragment key={heading}>
          {index > 0 && <DropdownMenuSeparator />}
          <DropdownMenuGroup aria-label={labels[heading]}>
            <DropdownMenuLabel>{labels[heading]}</DropdownMenuLabel>
            {actions.map(([label, command]) => <DropdownMenuItem key={command} disabled={!editor.can()[command]()}
              onSelect={() => { if (editor.isEditable && editor.isActive('table')) editor.chain().focus()[command]().run() }}>{labels[label]}</DropdownMenuItem>)}
          </DropdownMenuGroup>
        </Fragment>)}
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" disabled={!editor.can().deleteTable()}
          onSelect={() => { if (editor.isEditable && editor.isActive('table')) editor.chain().focus().deleteTable().run() }}>{labels.tableDelete}</DropdownMenuItem>
        <p className="text-muted-foreground px-2 py-1 text-xs">{labels.tableHint}</p>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
}
