import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Editor } from '@tiptap/react'
import * as Y from 'yjs'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import SectionEditor from './SectionEditor'
import { insertSlashBlock, slashMatch } from '@/lib/slash-insert'

const resources: (() => void)[] = []
afterEach(() => { cleanup(); resources.splice(0).forEach(destroy => destroy()) })
function mount(canWrite = true, onInsertSection = vi.fn(() => true)) {
  const doc = new Y.Doc(), awareness = new Awareness(doc)
  resources.push(() => { awareness.destroy(); doc.destroy() })
  let editor!: Editor
  render(<SectionEditor ydoc={doc} fragment={doc.getXmlFragment('prose')} provider={{ awareness } as unknown as WebsocketProvider}
    display="Ada" color="#2563eb" canWrite={canWrite} onSettled={() => {}} label="Prose"
    onInsertSection={onInsertSection} onEditor={value => { if (value) editor = value }} />)
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
  it('uses arrow navigation and keeps existing heading-to-section behavior', () => {
    const { editor, onInsertSection } = mount()
    type(editor, '/')
    key(editor, 'ArrowDown'); key(editor, 'Enter')
    expect(editor.state.doc.firstChild!.attrs.level).toBe(2)
    type(editor, 'Payments')
    key(editor, 'Enter')
    expect(onInsertSection).toHaveBeenCalledWith(2, 'Payments')
  })
  it('filters /h2 to one heading, shows its shortcut, and creates a section only once the title is complete', () => {
    const { editor, onInsertSection } = mount()
    type(editor, '/h2')
    expect(screen.getAllByRole('option')).toHaveLength(1)
    expect(screen.getByRole('option').textContent).toContain('Ctrl+Alt+2')
    key(editor, 'Enter')
    expect(editor.state.doc.firstChild!.attrs.level).toBe(2)
    expect(onInsertSection).not.toHaveBeenCalled()
    type(editor, 'Payments')
    key(editor, 'Enter')
    expect(onInsertSection).toHaveBeenCalledExactlyOnceWith(2, 'Payments')
    expect(editor.state.doc.textContent).toBe('')
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
    const { editor } = mount()
    type(editor, '## ')
    expect(editor.state.doc.firstChild!.type.name).toBe('heading')
    expect(editor.state.doc.firstChild!.attrs.level).toBe(2)
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
  it.each([
    ['plantuml', 'plantuml', '@startuml'],
    ['wireframe', 'plantuml', '@startsalt'],
    ['d2', 'd2', 'User -> Takomo'],
  ])('inserts a usable %s template', (command, language, source) => {
    const { editor } = mount()
    type(editor, `/${command}`); key(editor, 'Enter')
    expect(editor.state.doc.firstChild!.attrs.language).toBe(language)
    expect(editor.state.doc.firstChild!.textContent).toContain(source)
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
