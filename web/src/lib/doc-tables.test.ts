import { afterEach, describe, expect, it } from 'vitest'
import { Editor, getSchema } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import { CellSelection, TableMap } from '@tiptap/pm/tables'
import { prosemirrorJSONToYDoc, yDocToProsemirrorJSON } from 'y-prosemirror'
import * as Y from 'yjs'
import { annotatedMarkdown, BlockId } from './block-id'
import { applyOps, markdownToNodes } from './doc-ops'

const extensions = [StarterKit, TableKit.configure({ table: { resizable: true } }), BlockId.configure({ canWrite: true })]
const schema = getSchema(extensions)
const html = '<table><tr><th colspan="2" colwidth="140,180"><p><strong>Plan &amp; cost</strong></p></th></tr>\n\n<tr><td rowspan="2" colwidth="140"><p>Owner</p><ul><li><p><em>Alice</em></p></li></ul></td><td colwidth="180"><p>First</p></td></tr><tr><td colwidth="180"><p>Second</p><p><a href="https://example.com">Details</a></p></td></tr></table>'
let editor: Editor | undefined
afterEach(() => editor?.destroy())

describe('document tables', () => {
  it('preserves merged cells, headers, column widths and rich blocks through HTML and a remote CRDT replica', () => {
    const nodes = markdownToNodes(schema, html)
    expect(nodes).toHaveLength(1)
    const doc = schema.nodes.doc!.create(null, nodes)
    const source = prosemirrorJSONToYDoc(schema, doc.toJSON(), 'prose')
    const replica = new Y.Doc()
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(source))
    const restored = schema.nodeFromJSON(yDocToProsemirrorJSON(replica, 'prose'))
    expect(restored.toJSON()).toEqual(doc.toJSON())
    const roundtrip = markdownToNodes(schema, annotatedMarkdown(restored))
    expect(roundtrip[0]!.toJSON()).toEqual(nodes[0]!.toJSON())
    expect(TableMap.get(roundtrip[0]!).problems).toBeNull()
    source.destroy(); replica.destroy()
  })

  it('accepts pipe-table proposals as real tables and leaves adjacent blocks intact', () => {
    editor = new Editor({ extensions, content: '<p data-id="blk_before">Before</p><p data-id="blk_after">After</p>' })
    const tr = editor.state.tr
    applyOps(tr, editor.schema, [{ op: 'insert_after', id: 'blk_before', markdown: '| Name | Cost |\n| --- | ---: |\n| Alice | 10 |' }])
    editor.view.dispatch(tr)
    expect(editor.state.doc.child(1).type.name).toBe('table')
    expect(editor.state.doc.child(1).attrs.id).toMatch(/^blk_/)
    expect(editor.state.doc.child(1).child(1).child(1).attrs.align).toBe('right')
    expect(editor.state.doc.child(2).textContent).toBe('After')
  })

  it('inserts, merges, splits, changes headers, adds and removes rows and columns', () => {
    editor = new Editor({ extensions })
    expect(editor.commands.insertTable({ rows: 3, cols: 3, withHeaderRow: true })).toBe(true)
    const table = editor.state.doc.firstChild!
    const map = TableMap.get(table)
    editor.view.dispatch(editor.state.tr.setSelection(CellSelection.create(editor.state.doc, map.map[0]! + 1, map.map[1]! + 1)))
    expect(editor.commands.mergeCells()).toBe(true)
    expect(editor.state.doc.firstChild!.firstChild!.firstChild!.attrs.colspan).toBe(2)
    expect(editor.commands.splitCell()).toBe(true)
    expect(editor.commands.addRowAfter()).toBe(true)
    expect(editor.commands.addColumnAfter()).toBe(true)
    expect(TableMap.get(editor.state.doc.firstChild!).width).toBe(4)
    expect(editor.state.doc.firstChild!.childCount).toBe(4)
    expect(editor.commands.toggleHeaderRow()).toBe(true)
    expect(editor.commands.deleteRow()).toBe(true)
    expect(editor.commands.deleteColumn()).toBe(true)
    expect(TableMap.get(editor.state.doc.firstChild!).problems).toBeNull()
    expect(editor.commands.deleteTable()).toBe(true)
    expect(editor.state.doc.firstChild!.type.name).toBe('paragraph')
  })

  it('keeps fenced HTML literal and preserves blank lines in code blocks', () => {
    const nodes = markdownToNodes(schema, '```html\n<table>\n\n</table>\n```')
    expect(nodes).toHaveLength(1)
    expect(nodes[0]!.type.name).toBe('codeBlock')
    expect(nodes[0]!.textContent).toBe('<table>\n\n</table>')
  })
})
