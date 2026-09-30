import { afterEach, describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { DocumentSearchHighlight, setDocumentSearchHighlight } from './document-search-highlight'
import { HighlightBlocks, setHighlightedBlocks } from './block-highlight'
import { proseMatches } from './document-search'
import { LongTables, LongTableView } from './long-table-view'
import { longTableCut, longTableLabels } from './long-table'
import { BlockId } from './block-id'

/** A table with one header row and `body` body rows; cells read "r{n}". */
function tableHtml(body: number, { id = 'blk_t', rowspanAt }: { id?: string; rowspanAt?: { row: number; span: number } } = {}): string {
  const rows = ['<tr><th><p>Name</p></th><th><p>Value</p></th></tr>']
  for (let row = 1; row <= body; row++) {
    const merged = rowspanAt && row > rowspanAt.row && row < rowspanAt.row + rowspanAt.span
    const first = rowspanAt?.row === row ? `<td rowspan="${rowspanAt.span}"><p>m${row}</p></td>` : merged ? '' : `<td><p>r${row}</p></td>`
    rows.push(`<tr>${first}<td><p>v${row}</p></td></tr>`)
  }
  return `<p>Before</p><table data-id="${id}">${rows.join('')}</table><p>After</p>`
}

const editors: Editor[] = []
function create(content: string, { editable = true, locale = 'en' as 'en' | 'de', ydoc }: { editable?: boolean; locale?: 'en' | 'de'; ydoc?: Y.Doc } = {}) {
  const editor = new Editor({
    editable,
    extensions: [
      StarterKit.configure(ydoc ? { undoRedo: false } : {}),
      TableKit.configure({ table: { resizable: true, View: LongTableView } }),
      LongTables.configure({ labels: () => longTableLabels(locale) }),
      DocumentSearchHighlight,
      HighlightBlocks,
      BlockId.configure({ canWrite: true }),
      ...(ydoc ? [Collaboration.configure({ document: ydoc, field: 'prose' })] : []),
    ],
    ...(ydoc ? {} : { content }),
  })
  if (ydoc) editor.commands.setContent(content)
  editors.push(editor)
  return editor
}
afterEach(() => editors.splice(0).forEach(editor => editor.destroy()))

const wrapper = (editor: Editor) => editor.view.dom.querySelector<HTMLElement>('.tableWrapper')!
const toggle = (editor: Editor) => wrapper(editor).querySelector<HTMLButtonElement>('.document-long-table-toggle')!
const hiddenRule = (editor: Editor) => wrapper(editor).querySelector('style')!.textContent
function textPos(editor: Editor, text: string): number {
  let found = -1
  editor.state.doc.descendants((node, pos) => { if (found < 0 && node.isText && node.text === text) found = pos })
  return found
}

describe('long document tables (editor)', () => {
  it('leaves a table with ten body rows alone and collapses one with eleven', () => {
    const ten = create(tableHtml(10))
    expect(wrapper(ten).dataset.longTable).toBeUndefined()
    expect(wrapper(ten).querySelector<HTMLElement>('.document-long-table-controls')!.hidden).toBe(true)
    expect(hiddenRule(ten)).toBe('')

    const eleven = create(tableHtml(11))
    expect(wrapper(eleven).dataset.longTable).toBe('collapsed')
    expect(toggle(eleven).textContent).toBe('Show all 11 rows')
    expect(toggle(eleven).getAttribute('aria-expanded')).toBe('false')
    // Header row + ten body rows stay; rows 12.. are hidden.
    expect(hiddenRule(eleven)).toContain('tr:nth-child(n+12)')
    const table = wrapper(eleven).querySelector('table')!
    expect(toggle(eleven).getAttribute('aria-controls')).toBe(table.id)
    expect(document.getElementById(table.getAttribute('aria-describedby')!) ?? wrapper(eleven).querySelector(`#${table.getAttribute('aria-describedby')}`)).toHaveProperty('textContent', 'Showing 10 of 11 rows')
  })

  it('hides the rows from layout (and so from Tab) only while collapsed', () => {
    const editor = create(tableHtml(12))
    document.body.append(editor.view.dom)
    const rows = wrapper(editor).querySelectorAll('tbody > tr')
    expect(getComputedStyle(rows[10]!).display).not.toBe('none')
    expect(getComputedStyle(rows[11]!).display).toBe('none')
    toggle(editor).click()
    expect(getComputedStyle(rows[11]!).display).not.toBe('none')
    editor.view.dom.remove()
  })

  it('toggles open and closed with the right labels, in German too', () => {
    const editor = create(tableHtml(15), { locale: 'de' })
    expect(toggle(editor).textContent).toBe('Alle 15 Zeilen anzeigen')
    toggle(editor).click()
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
    expect(toggle(editor).textContent).toBe('Weniger anzeigen')
    expect(toggle(editor).getAttribute('aria-expanded')).toBe('true')
    expect(hiddenRule(editor)).toBe('')
    expect(wrapper(editor).querySelector('table')!.hasAttribute('aria-describedby')).toBe(false)
    toggle(editor).click()
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
  })

  it('never writes to the shared document: the Y.Doc state vector is unchanged and nothing enters undo', () => {
    const ydoc = new Y.Doc()
    const editor = create(tableHtml(20), { ydoc })
    const before = Y.encodeStateVector(ydoc)
    const json = JSON.stringify(editor.getJSON())
    const updates: Uint8Array[] = []
    ydoc.on('update', update => updates.push(update))
    toggle(editor).click()
    toggle(editor).click()
    toggle(editor).click()
    expect(Y.encodeStateVector(ydoc)).toEqual(before)
    expect(updates).toHaveLength(0)
    expect(JSON.stringify(editor.getJSON())).toBe(json)
    expect(JSON.stringify(editor.getJSON())).not.toContain('expanded')
    ydoc.destroy()
  })

  it('stays expanded across document updates, and a fresh mount starts collapsed', () => {
    const editor = create(tableHtml(12))
    toggle(editor).click()
    editor.commands.insertContentAt(1, 'Edited ')
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
    const again = create(editor.getHTML())
    expect(wrapper(again).dataset.longTable).toBe('collapsed')
  })

  it('expands when the caret moves into a hidden row, but not for a visible one', () => {
    const editor = create(tableHtml(14))
    editor.commands.setTextSelection(textPos(editor, 'r5') + 1)
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
    editor.commands.setTextSelection(textPos(editor, 'r13') + 1)
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
    // Collapsing again with the caret still there is respected until it moves.
    toggle(editor).click()
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
  })

  it('expands for an active search hit in a hidden row only', () => {
    const editor = create(tableHtml(14))
    const visible = proseMatches(editor.state.doc, 'v3')[0]!
    setDocumentSearchHighlight(editor.view, { query: 'v3', activeFrom: visible.from })
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
    const hidden = proseMatches(editor.state.doc, 'v12')[0]!
    setDocumentSearchHighlight(editor.view, { query: 'v12', activeFrom: hidden.from })
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
    expect(editor.state.selection.from).toBe(1)
  })

  it('expands when a pending proposal highlights the table', () => {
    const editor = create(tableHtml(14))
    setHighlightedBlocks(editor.view, new Set(['blk_other']))
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
    setHighlightedBlocks(editor.view, new Set(['blk_t']))
    expect(wrapper(editor).classList.contains('takomo-proposed')).toBe(true)
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
  })

  it('cuts at a row boundary no rowspan crosses', () => {
    // Body row 9 spans rows 9–12: the first clean boundary after row 10 is after row 12.
    const editor = create(tableHtml(16, { rowspanAt: { row: 9, span: 4 } }))
    expect(hiddenRule(editor)).toContain('tr:nth-child(n+14)')
    const table = wrapper(editor).querySelector('table')!
    expect(wrapper(editor).querySelector(`#${table.getAttribute('aria-describedby')}`)!.textContent).toBe('Showing 12 of 16 rows')
  })

  it('works the same in a read-only editor', () => {
    const editor = create(tableHtml(11), { editable: false })
    expect(editor.isEditable).toBe(false)
    expect(wrapper(editor).dataset.longTable).toBe('collapsed')
    toggle(editor).click()
    expect(wrapper(editor).dataset.longTable).toBe('expanded')
  })
})

describe('longTableCut', () => {
  const rows = (body: number, extra: Record<number, number> = {}) => [
    { header: true, rowspans: [1, 1] },
    ...Array.from({ length: body }, (_, index) => ({ header: false, rowspans: [extra[index + 1] ?? 1, 1] })),
  ]
  it('collapses only above ten body rows, not counting header rows', () => {
    expect(longTableCut(rows(10))).toBeNull()
    expect(longTableCut(rows(11))).toEqual({ headerRows: 1, bodyRows: 11, visibleRows: 11, visibleBodyRows: 10 })
    expect(longTableCut(Array.from({ length: 11 }, () => ({ header: false, rowspans: [1] })))).toMatchObject({ headerRows: 0, visibleRows: 10 })
  })
  it('moves the cut past a rowspan and gives up when no boundary is left', () => {
    expect(longTableCut(rows(14, { 10: 2 }))).toMatchObject({ visibleRows: 12, visibleBodyRows: 11 })
    expect(longTableCut(rows(11, { 10: 2 }))).toBeNull()
  })
})
