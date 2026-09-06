import { useEditorState, type Editor } from '@tiptap/react'

export interface TableLabels {
  tableInsert: string; tableRowBefore: string; tableRowAfter: string; tableRowDelete: string
  tableColumnBefore: string; tableColumnAfter: string; tableColumnDelete: string
  tableHeaderRow: string; tableHeaderColumn: string; tableMerge: string; tableSplit: string
  tableDelete: string; tableHint: string
}

/** React follows transactions too: merging and remote edits change which actions are possible. */
export function TableToolbar({ editor, labels, disabled }: { editor: Editor; labels: TableLabels; disabled: boolean }) {
  useEditorState({ editor, selector: ({ transactionNumber }) => transactionNumber })
  const inside = editor.isActive('table')
  const actions = [
    ['tableRowBefore', 'addRowBefore'], ['tableRowAfter', 'addRowAfter'],
    ['tableColumnBefore', 'addColumnBefore'], ['tableColumnAfter', 'addColumnAfter'],
    ['tableHeaderRow', 'toggleHeaderRow'], ['tableHeaderColumn', 'toggleHeaderColumn'],
    ['tableMerge', 'mergeCells'], ['tableSplit', 'splitCell'],
    ['tableRowDelete', 'deleteRow'], ['tableColumnDelete', 'deleteColumn'], ['tableDelete', 'deleteTable'],
  ] as const
  const buttonClass = 'rounded border border-border-soft px-2 py-1 text-xs hover:bg-accent disabled:opacity-40'
  return <div className="mb-3 flex flex-wrap items-center gap-1.5" role="group" aria-label={labels.tableInsert}>
    <button type="button" className={buttonClass} disabled={disabled || inside}
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}>{labels.tableInsert}</button>
    {inside && actions.map(([label, command]) => <button key={command} type="button" className={buttonClass}
      disabled={disabled || !editor.can()[command]()}
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => editor.chain().focus()[command]().run()}>{labels[label]}</button>)}
    {inside && <p className="text-muted-foreground basis-full text-xs">{labels.tableHint}</p>}
  </div>
}
