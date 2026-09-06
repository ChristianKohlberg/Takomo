import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Editor } from '@tiptap/react'
import * as Y from 'yjs'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import SectionEditor from './SectionEditor'
import { insertSlashSection, insertSlashBlock, slashMatch } from '@/lib/slash-insert'

import { createNode, readPlanTree } from '@/lib/mindmap-crdt'
import { insertPlanSection } from '@/lib/plan-insert'

const resources: (() => void)[] = []
afterEach(() => { cleanup(); resources.splice(0).forEach(destroy => destroy()) })
function mount(canWrite = true, onInsertSection = vi.fn(() => true), maxSectionLevel = 3) {
  const doc = new Y.Doc(), awareness = new Awareness(doc)
  resources.push(() => { awareness.destroy(); doc.destroy() })
  let editor!: Editor
  render(<SectionEditor ydoc={doc} fragment={doc.getXmlFragment('prose')} provider={{ awareness } as unknown as WebsocketProvider}
    display="Ada" color="#2563eb" canWrite={canWrite} onSettled={() => {}} label="Prose"
    maxSectionLevel={maxSectionLevel} onInsertSection={onInsertSection} onEditor={value => { if (value) editor = value }} />)
  vi.spyOn(editor.view, 'coordsAtPos').mockReturnValue({ left: 20, right: 20, top: 20, bottom: 40 })
  return { editor, doc, onInsertSection }
}
function type(editor: Editor, text: string) {
  act(() => {
    for (const letter of text) {
      const { from, to } = editor.state.selection
      const handled = editor.view.someProp('handleTextInput', handler => handler(editor.view, from, to, letter, () => editor.state.tr.insertText(letter)))
      if (!handled) editor.view.dispatch(editor.state.tr.insertText(letter, from, to))
    }
  })
}
function key(editor: Editor, key: string) { fireEvent.keyDown(editor.view.dom, { key }) }

describe('slash insertion in a collaborative section', () => {
  it('filters commands and inserts a quotation with keyboard focus', () => {
    const { editor } = mount()
    type(editor, '/quote')
    expect(screen.getAllByRole('option')).toHaveLength(1)
    key(editor, 'Enter')
    expect(editor.state.doc.firstChild!.type.name).toBe('blockquote')
    expect(editor.state.doc.textContent).toBe('')
    expect(screen.queryByRole('listbox')).toBeNull()
  })
  it('uses arrow navigation to request a canonical section title', () => {
    const { editor, onInsertSection } = mount()
    type(editor, '/')
    key(editor, 'ArrowDown'); key(editor, 'Enter')
    expect(screen.getByRole('dialog', { name: 'New section H2' })).toBeTruthy()
    expect(onInsertSection).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Section title'), { target: { value: 'Payments' } })
    fireEvent.submit(screen.getByRole('dialog'))
    expect(onInsertSection).toHaveBeenCalledExactlyOnceWith(2, 'Payments')
    expect(editor.state.doc.textContent).toBe('')
    expect(editor.state.doc.firstChild!.type.name).toBe('paragraph')
  })
  it('keeps surrounding prose when /h2 creates a real nested section', () => {
    const { editor, doc } = mount()
    const parent = createNode(doc, { parent: null, title: 'Parent', by: 'Ada' })!
    const insert = vi.fn((level: 1 | 2 | 3, title: string) => !!insertPlanSection(doc, parent, level, title, 'Ada'))
    // Exercise the same canonical callback used by Plan, at a middle paragraph.
    act(() => { editor.commands.setContent('<p>Before</p><p></p><p>After</p>'); editor.commands.setTextSelection(9) })
    type(editor, '/h2')
    const match = slashMatch(editor.state)!
    act(() => expect(insertSlashSection(editor, match, 2, 'Child', insert)).toBe(true))
    expect(editor.state.doc.textContent).toBe('BeforeAfter')
    expect(readPlanTree(doc).find(node => node.title === 'Child')?.parent).toBe(parent)
    expect(insertSlashSection(editor, match, 2, 'Duplicate', insert)).toBe(false)
    expect(insert).toHaveBeenCalledTimes(1)
  })
  it('keeps the query on cancellation and the title on a refused insertion', () => {
    const { editor, onInsertSection } = mount(true, vi.fn(() => false))
    type(editor, '/h2'); key(editor, 'Enter')
    expect((screen.getByRole('button', { name: 'Create section' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Section title'), { target: { value: 'Payments' } })
    fireEvent.submit(screen.getByRole('dialog'))
    expect(screen.getByRole('alert')).toBeTruthy()
    expect((screen.getByLabelText('Section title') as HTMLInputElement).value).toBe('Payments')
    expect(editor.state.doc.textContent).toBe('/h2')
    expect(onInsertSection).toHaveBeenCalledTimes(1)
    fireEvent.keyDown(screen.getByLabelText('Section title'), { key: 'Escape' })
    expect(screen.getByRole('listbox')).toBeTruthy()
    key(editor, 'Escape')
    expect(editor.state.doc.textContent).toBe('/h2')
  })
  it('keeps unavailable levels discoverable and refuses skipped hierarchy levels', () => {
    const { editor, onInsertSection } = mount(true, vi.fn(() => true), 2)
    type(editor, '/h3'); key(editor, 'Enter')
    expect(screen.getByRole('option').getAttribute('aria-disabled')).toBe('true')
    expect(screen.getByText('A parent section is required at this boundary.')).toBeTruthy()
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(onInsertSection).not.toHaveBeenCalled()
  })
  it('formats selected prose with the heading shortcut without splitting content', () => {
    const { editor, onInsertSection } = mount()
    act(() => {
      editor.commands.setContent('<p>Keep this title</p><p>Keep this paragraph.</p>')
      editor.commands.setTextSelection({ from: 1, to: 16 })
      editor.commands.keyboardShortcut('Mod-Alt-2')
    })
    expect(editor.state.doc.firstChild!.attrs.level).toBe(2)
    expect(editor.state.doc.textContent).toBe('Keep this titleKeep this paragraph.')
    expect(onInsertSection).not.toHaveBeenCalled()
    act(() => editor.commands.setTextSelection(16))
    key(editor, 'Enter')
    expect(onInsertSection).not.toHaveBeenCalled()
    expect(editor.state.doc.textContent).toContain('Keep this paragraph.')
  })
  it('chooses table dimensions and returns to the menu on picker cancellation', () => {
    const { editor } = mount()
    type(editor, '/tab'); key(editor, 'Enter')
    expect(screen.getByLabelText('Rows')).toBeTruthy()
    fireEvent.keyDown(screen.getByLabelText('Rows'), { key: 'Escape' })
    expect(screen.getByRole('listbox')).toBeTruthy()
    expect(editor.state.doc.textContent).toBe('/tab')
    key(editor, 'Enter')
    fireEvent.change(screen.getByLabelText('Rows'), { target: { value: '2' } })
    fireEvent.change(screen.getByLabelText('Columns'), { target: { value: '4' } })
    fireEvent.click(screen.getAllByRole('button', { name: 'Insert table' }).at(-1)!)
    expect(editor.state.doc.firstChild!.type.name).toBe('table')
    expect(editor.state.doc.firstChild!.childCount).toBe(2)
    expect(editor.state.doc.firstChild!.firstChild!.childCount).toBe(4)
    expect(editor.isActive('table')).toBe(true)
  })
  it('Escape and unmatched results preserve the typed text', () => {
    const { editor } = mount()
    type(editor, '/not-a-command')
    expect(screen.getByRole('status').textContent).toBe('No matching blocks.')
    key(editor, 'Enter'); key(editor, 'Escape')
    expect(editor.state.doc.textContent).toBe('/not-a-command')
    expect(screen.queryByRole('listbox')).toBeNull()
    type(editor, ' more')
    expect(screen.queryByRole('listbox')).toBeNull()
  })
  it('leaves ordinary slashes, code and read-only content alone', () => {
    const { editor } = mount()
    type(editor, 'Use /tmp/example')
    expect(screen.queryByRole('listbox')).toBeNull()
    act(() => { editor.commands.setContent('<pre><code></code></pre>'); editor.commands.focus('start') })
    type(editor, '/')
    expect(screen.queryByRole('listbox')).toBeNull()
    act(() => { editor.commands.setContent('<p></p>'); editor.setEditable(false) })
    type(editor, '/')
    expect(screen.queryByRole('listbox')).toBeNull()
  })
  it('keeps Markdown heading shortcuts and does not open for loaded slash text', () => {
    const { editor, onInsertSection } = mount()
    type(editor, '## ')
    expect(editor.state.doc.firstChild!.type.name).toBe('heading')
    expect(editor.state.doc.firstChild!.attrs.level).toBe(2)
    type(editor, 'Existing shortcut')
    key(editor, 'Enter')
    expect(onInsertSection).toHaveBeenCalledExactlyOnceWith(2, 'Existing shortcut')
    act(() => editor.commands.setContent('<p>/table</p>'))
    expect(screen.queryByRole('listbox')).toBeNull()
  })
  it('inserts the supported Mermaid block with editable source', () => {
    const { editor } = mount()
    type(editor, '/mermaid'); key(editor, 'Enter')
    expect(editor.state.doc.firstChild!.type.name).toBe('codeBlock')
    expect(editor.state.doc.firstChild!.attrs.language).toBe('mermaid')
    expect(editor.view.dom.querySelector('pre')!.hidden).toBe(false)
    expect(screen.queryByRole('option', { name: /wireframe/i })).toBeNull()
  })
  it('rejects a stale insertion range after another peer changes the trigger', () => {
    const { editor, doc } = mount()
    type(editor, '/table')
    const captured = slashMatch(editor.state)!
    const replica = new Y.Doc()
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(doc))
    const paragraph = replica.getXmlFragment('prose').get(0) as Y.XmlElement
    const text = paragraph.get(0) as Y.XmlText
    text.insert(1, 'changed-')
    act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(replica)))
    const insert = vi.fn(() => true)
    expect(insertSlashSection(editor, captured, 2, 'Stale section', insert)).toBe(false)
    expect(insert).not.toHaveBeenCalled()
    expect(insertSlashBlock(editor, captured, 'table', 2, 4)).toBe(false)
    expect(editor.state.doc.textContent).toContain('changed-')
    expect(editor.state.doc.firstChild!.type.name).toBe('paragraph')
    replica.destroy()
  })
  it('exposes collaborative undo and redo without undoing a peer edit', () => {
    const { editor, doc } = mount()
    type(editor, 'Local')
    const replica = new Y.Doc()
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(doc))
    const paragraph = replica.getXmlFragment('prose').get(0) as Y.XmlElement
    const text = paragraph.get(0) as Y.XmlText
    text.insert(text.length, ' peer')
    act(() => Y.applyUpdate(doc, Y.encodeStateAsUpdate(replica)))
    expect(editor.can().undo()).toBe(true)
    act(() => { editor.commands.undo() })
    expect(editor.state.doc.textContent).toBe(' peer')
    expect(editor.can().redo()).toBe(true)
    act(() => { editor.commands.redo() })
    expect(editor.state.doc.textContent).toBe('Local peer')
    replica.destroy()
  })
})
