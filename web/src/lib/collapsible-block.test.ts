import { afterEach, describe, expect, it, vi } from 'vitest'
import { Editor, getSchema } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import Collaboration from '@tiptap/extension-collaboration'
import { prosemirrorJSONToYDoc, yDocToProsemirrorJSON } from 'y-prosemirror'
import * as Y from 'yjs'
import { DocumentSearchHighlight, setDocumentSearchHighlight } from './document-search-highlight'
import { CollapsibleExtensions } from './collapsible-block'
import { annotatedMarkdown, BlockId } from './block-id'
import { markdownToNodes } from './doc-ops'
import { fragmentText } from './mindmap-crdt'
import { fragmentMatches, proseMatches } from './document-search'

const extensions = [StarterKit, TableKit, ...CollapsibleExtensions, DocumentSearchHighlight, BlockId.configure({ canWrite: true })]
const schema = getSchema(extensions)
const html = '<details data-collapsible-block><summary>Permissions</summary><div data-collapsible-content><table><tr><th><p>Capability</p></th></tr><tr><td><p><code>patron.update</code></p></td></tr></table><p>Context</p></div></details>'
const editors: Editor[] = []
const create = (content = html) => { const editor = new Editor({ extensions, content }); editors.push(editor); return editor }
afterEach(() => editors.splice(0).forEach(e => e.destroy()))

describe('collapsible document content', () => {
  it('keeps tables, inline code and titles through HTML, agent markdown and a fresh CRDT replica', () => {
    const source = create()
    const exported = annotatedMarkdown(source.state.doc)
    const parsed = markdownToNodes(schema, exported)
    expect(parsed).toHaveLength(1)
    expect(parsed[0]!.type.name).toBe('collapsibleBlock')
    expect(parsed[0]!.child(1).firstChild!.type.name).toBe('table')
    expect(parsed[0]!.textContent).toBe('PermissionsCapabilitypatron.updateContext')
    const doc = prosemirrorJSONToYDoc(schema, source.getJSON(), 'prose')
    const replica = new Y.Doc()
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(doc))
    const restored = yDocToProsemirrorJSON(replica, 'prose')
    expect(schema.nodeFromJSON(restored).toJSON()).toEqual(source.getJSON())
    const restoredEditor = create(); restoredEditor.commands.setContent(restored)
    expect(restoredEditor.view.dom.querySelector('code')!.textContent).toBe('patron.update')
    expect(fragmentText(replica.getXmlFragment('prose'))).toContain('Permissions')
    expect(fragmentMatches(replica.getXmlFragment('prose'), 'Permissions').map(({ from, to }) => ({ from, to }))).toEqual(proseMatches(source.state.doc, 'Permissions'))
    expect(fragmentMatches(replica.getXmlFragment('prose'), 'patron.update').map(({ from, to }) => ({ from, to }))).toEqual(proseMatches(source.state.doc, 'patron.update'))
    doc.destroy(); replica.destroy()
  })
  it('wraps the whole table from inside a cell and unwraps without losing content or the table ID', () => {
    const editor = create('<p>Before</p><table data-id="blk_table"><tr><td><p>Value</p></td></tr></table><p>After</p>')
    editor.commands.setTextSelection(12)
    expect(editor.isActive('table')).toBe(true)
    expect(editor.commands.wrapCollapsibleBlock('Permissions')).toBe(true)
    expect(editor.state.doc.child(1).type.name).toBe('collapsibleBlock')
    expect(editor.state.doc.child(1).child(1).firstChild!.attrs.id).toBe('blk_table')
    expect(editor.commands.unwrapCollapsibleBlock()).toBe(true)
    expect(editor.state.doc.child(1).type.name).toBe('table')
    expect(editor.state.doc.child(1).attrs.id).toBe('blk_table')
    expect(editor.state.doc.textContent).toBe('BeforeValueAfter')
  })
  it('starts closed, toggles without changing shared data, and disallows mutation for readers', () => {
    const seed = create()
    const doc = prosemirrorJSONToYDoc(schema, seed.getJSON(), 'prose')
    const reader = new Editor({ editable: false, extensions: [StarterKit.configure({ undoRedo: false }), TableKit, ...CollapsibleExtensions, Collaboration.configure({ fragment: doc.getXmlFragment('prose') })] })
    editors.push(reader)
    const updates: Uint8Array[] = []; doc.on('update', value => updates.push(value))
    const details = reader.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!
    expect(details.dataset.expanded).toBe('false')
    details.querySelector<HTMLButtonElement>('.document-collapsible-toggle')!.click()
    details.querySelector<HTMLButtonElement>('.document-collapsible-toggle')!.click()
    expect(updates).toHaveLength(0)
    expect(reader.commands.wrapCollapsibleBlock()).toBe(false)
    reader.commands.setTextSelection(2)
    expect(reader.commands.unwrapCollapsibleBlock()).toBe(false)
    expect(reader.view.dom.querySelector<HTMLElement>('.document-collapsible-controls')!.hidden).toBe(true)
    reader.destroy(); editors.splice(editors.indexOf(reader), 1); doc.destroy()
  })
  it('moves from the summary into its body with Enter', () => {
    const editor = create('<p>Body</p>')
    editor.commands.wrapCollapsibleBlock('Heading')
    const handled = editor.view.someProp('handleKeyDown', handler => handler(editor.view, new KeyboardEvent('keydown', { key: 'Enter' })))
    expect(handled).toBe(true)
    expect(editor.state.selection.$from.parent.textContent).toBe('Body')
    expect(editor.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!.dataset.expanded).toBe('true')
  })
  it('Enter reaches the first table cell when the body starts with a table', () => {
    const editor = create()
    editor.commands.setTextSelection(2)
    editor.view.someProp('handleKeyDown', handler => handler(editor.view, new KeyboardEvent('keydown', { key: 'Enter' })))
    expect(editor.state.selection.$from.parent.type.name).toBe('paragraph')
    expect(editor.state.selection.$from.parent.textContent).toBe('Capability')
    expect(editor.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!.dataset.expanded).toBe('true')
  })
  it('reveals an active in-document search decoration without changing content or selection', () => {
    const editor = create()
    editor.view.dispatch(editor.state.tr)
    const original = editor.getJSON()
    const selection = editor.state.selection.from
    const match = proseMatches(editor.state.doc, 'patron.update')[0]!
    const details = editor.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!
    expect(details.dataset.expanded).toBe('false')
    setDocumentSearchHighlight(editor.view, { query: 'patron.update', activeFrom: match.from })
    expect(details.dataset.expanded).toBe('true')
    expect(editor.getJSON()).toEqual(original)
    expect(editor.state.selection.from).toBe(selection)
  })
  it('reveals a search passage spanning the summary and body', () => {
    const editor = create()
    const block = editor.state.doc.firstChild!
    editor.commands.setTextSelection({ from: 2, to: block.nodeSize - 2 })
    expect(editor.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!.dataset.expanded).toBe('true')
  })
  it('clears a stale whole-passage selection before typing in title padding', () => {
    const editor = create()
    const block = editor.state.doc.firstChild!
    editor.commands.setTextSelection({ from: 2, to: block.nodeSize - 2 })
    vi.spyOn(editor.view, 'posAtCoords').mockReturnValue({ pos: block.nodeSize - 2, inside: 0 })
    editor.view.dom.querySelector('.document-collapsible-summary')!.dispatchEvent(new MouseEvent('mousedown', { button: 0 }))
    expect(editor.state.selection.empty).toBe(true)
    expect(editor.state.selection.$from.parent.type.name).toBe('collapsibleSummary')
    editor.commands.insertContent(' revised')
    expect(editor.state.doc.firstChild!.child(1).firstChild!.type.name).toBe('table')
    expect(editor.view.dom.querySelector('td code')!.textContent).toBe('patron.update')
  })
  it('supports undo and reveals content selected by search without storing open state', () => {
    const editor = create('<p>Search me</p>')
    editor.commands.wrapCollapsibleBlock()
    const details = editor.view.dom.querySelector<HTMLElement>('[data-collapsible-block]')!
    expect(details.dataset.expanded).toBe('false')
    let pos = 0; editor.state.doc.descendants((node, position) => { if (node.isText && node.text === 'Search me') pos = position })
    editor.commands.setTextSelection(pos)
    expect(details.dataset.expanded).toBe('true')
    expect(editor.getHTML()).not.toContain('open=')
    editor.commands.undo()
    expect(editor.state.doc.firstChild!.type.name).toBe('paragraph')
    expect(editor.state.doc.textContent).toBe('Search me')
  })
})
