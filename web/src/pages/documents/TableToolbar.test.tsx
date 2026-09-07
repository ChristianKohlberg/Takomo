import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import { CellSelection, TableMap } from '@tiptap/pm/tables'
import * as Y from 'yjs'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import SectionEditor from './SectionEditor'

const resources: (() => void)[] = []
afterEach(() => { cleanup(); resources.splice(0).forEach(destroy => destroy()) })
function mount(canWrite = true) {
  const doc = new Y.Doc(), awareness = new Awareness(doc)
  resources.push(() => { awareness.destroy(); doc.destroy() })
  let editor!: Editor
  render(<><button>Elsewhere</button><SectionEditor ydoc={doc} fragment={doc.getXmlFragment('prose')}
    provider={{ awareness } as unknown as WebsocketProvider} display="Ada" color="#2563eb" canWrite={canWrite}
    onSettled={() => {}} label="Prose" onEditor={value => { if (value) editor = value }} /></>)
  return editor
}
function table(editor: Editor) {
  act(() => {
    editor.commands.insertTable({ rows: 2, cols: 2, withHeaderRow: true })
    editor.view.dom.focus()
  })
}
function open() { fireEvent.keyDown(screen.getByRole('button', { name: 'Table actions' }), { key: 'ArrowDown' }) }

describe('contextual table actions', () => {
  it('shows no insertion button, and hides all actions outside the active table', () => {
    const editor = mount()
    expect(screen.queryByRole('button', { name: 'Insert table' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Table actions' })).toBeNull()
    table(editor)
    expect(screen.getByRole('button', { name: 'Table actions' })).toBeTruthy()
    expect(screen.queryByRole('menuitem')).toBeNull()
    act(() => screen.getByRole('button', { name: 'Elsewhere' }).focus())
    expect(screen.queryByRole('button', { name: 'Table actions' })).toBeNull()
    act(() => { editor.commands.setContent('<p>Outside</p>'); editor.view.dom.focus() })
    expect(screen.queryByRole('button', { name: 'Table actions' })).toBeNull()
  })
  it('opens grouped keyboard actions and applies a row action to the preserved selection', async () => {
    const editor = mount(); table(editor)
    const selected = editor.state.selection.from
    open()
    expect(screen.getByRole('group', { name: 'Rows' })).toBeTruthy()
    expect(screen.getByRole('group', { name: 'Columns' })).toBeTruthy()
    expect(screen.getByRole('group', { name: 'Headers and cells' })).toBeTruthy()
    expect(screen.getByRole('menuitem', { name: 'Merge cells' }).getAttribute('data-disabled')).not.toBeNull()
    expect(editor.state.selection.from).toBe(selected)
    fireEvent.click(screen.getByRole('menuitem', { name: 'Row after' }))
    expect(editor.state.doc.firstChild!.childCount).toBe(3)
    await waitFor(() => expect(document.activeElement).toBe(editor.view.dom))
    expect(screen.queryByRole('menu')).toBeNull()
  })
  it('preserves a rectangular selection for merge and exposes split afterwards', async () => {
    const editor = mount(); table(editor)
    const map = TableMap.get(editor.state.doc.firstChild!)
    act(() => editor.view.dispatch(editor.state.tr.setSelection(CellSelection.create(editor.state.doc, map.map[0]! + 1, map.map[1]! + 1))))
    open()
    fireEvent.click(screen.getByRole('menuitem', { name: 'Merge cells' }))
    expect(editor.state.doc.firstChild!.firstChild!.firstChild!.attrs.colspan).toBe(2)
    await waitFor(() => expect(document.activeElement).toBe(editor.view.dom))
    open()
    fireEvent.click(screen.getByRole('menuitem', { name: 'Split cell' }))
    expect(editor.state.doc.firstChild!.firstChild!.childCount).toBe(2)
    expect(TableMap.get(editor.state.doc.firstChild!).problems).toBeNull()
  })
  it('Escape restores table focus and outside focus stays outside', async () => {
    const editor = mount(); table(editor); open()
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' })
    await waitFor(() => expect(document.activeElement).toBe(editor.view.dom))
    open()
    act(() => screen.getByRole('button', { name: 'Elsewhere' }).focus())
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull())
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Elsewhere' }))
  })
  it('deleting a table removes the contextual controls, and read-only has none', () => {
    const editor = mount(); table(editor); open()
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete table' }))
    expect(editor.isActive('table')).toBe(false)
    expect(screen.queryByRole('button', { name: 'Table actions' })).toBeNull()
    cleanup()
    const readonly = mount(false); table(readonly)
    expect(screen.queryByRole('button', { name: 'Table actions' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Insert table' })).toBeNull()
  })
})
