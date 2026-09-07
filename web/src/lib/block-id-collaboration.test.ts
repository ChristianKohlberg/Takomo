import { afterEach, describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import { yUndoPluginKey } from '@tiptap/y-tiptap'
import * as Y from 'yjs'
import { BlockId } from './block-id'
import { CollaborationHistorySelection } from './collaboration-history-selection'

const editors: Editor[] = []
const docs: Y.Doc[] = []
afterEach(() => { editors.splice(0).forEach(e => e.destroy()); docs.splice(0).forEach(d => d.destroy()) })
function setup(canWrite = true, initialId: string | null = 'blk_original') {
  const doc = new Y.Doc(); docs.push(doc)
  const fragment = doc.getXmlFragment('prose')
  const p = new Y.XmlElement('paragraph'); if (initialId) p.setAttribute('id', initialId); p.insert(0, [new Y.XmlText('Hello world')]); fragment.insert(0, [p])
  const editor = new Editor({ extensions: [StarterKit.configure({ undoRedo: false }), Collaboration.configure({ document: doc, fragment }), BlockId.configure({ canWrite }), CollaborationHistorySelection] })
  editors.push(editor)
  const history = yUndoPluginKey.getState(editor.state).undoManager as Y.UndoManager
  return { doc, fragment, editor, history }
}
const ids = (editor: Editor) => { const result: string[] = []; editor.state.doc.forEach(n => result.push(n.attrs.id)); return result }

describe('collaborative structural undo', () => {
  it('undoes Enter in one step and redoes the same block IDs', () => {
    const { editor } = setup()
    editor.commands.setTextSelection(6)
    editor.commands.enter()
    expect(editor.state.doc.childCount).toBe(2)
    const splitIds = ids(editor)
    expect(new Set(splitIds).size).toBe(2)
    expect(splitIds[0]).toBe('blk_original')
    expect(editor.can().undo()).toBe(true)
    editor.commands.undo()
    expect(editor.getText()).toBe('Hello world')
    expect(ids(editor)).toEqual(['blk_original'])
    editor.commands.redo()
    expect(ids(editor)).toEqual(splitIds)
    expect(editor.getText()).toBe('Hello\n\n world')
  })
  it('undoes multi-block HTML paste as one action and preserves IDs through redo', () => {
    const { editor } = setup()
    editor.commands.setTextSelection(12)
    editor.view.pasteHTML('<p>First pasted</p><p>Second pasted</p><ul><li>List text</li></ul>', new Event('paste') as ClipboardEvent)
    const pasted = editor.getJSON()
    const pastedSelection = editor.state.selection.toJSON()
    expect(editor.state.doc.childCount).toBeGreaterThan(2)
    expect(new Set(ids(editor)).size).toBe(editor.state.doc.childCount)
    editor.commands.undo()
    expect(editor.getText()).toBe('Hello world')
    expect(editor.state.selection.anchor).toBe(12)
    expect(ids(editor)).toEqual(['blk_original'])
    editor.commands.redo()
    expect(editor.getJSON()).toEqual(pasted)
    expect(editor.state.selection.toJSON()).toEqual(pastedSelection)
    editor.commands.undo()
    editor.commands.redo()
    expect(editor.getJSON()).toEqual(pasted)
    expect(editor.state.selection.toJSON()).toEqual(pastedSelection)
  })
  it('keeps a remote collaborator edit when undoing a local split and subsequent typing', () => {
    const { doc, fragment, editor, history } = setup()
    editor.commands.setTextSelection(6)
    editor.commands.enter()
    history.stopCapturing()
    editor.commands.insertContent('local ')
    const peer = new Y.Doc(); docs.push(peer); Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    const peerText = (peer.getXmlFragment('prose').get(0) as Y.XmlElement).get(0) as Y.XmlText
    peerText.insert(0, 'Remote ')
    Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer))
    editor.commands.undo()
    expect(editor.getText()).toBe('Remote Hello\n\n world')
    editor.commands.undo()
    expect(editor.getText()).toBe('Remote Hello world')
    expect(editor.state.selection.anchor).toBe(13)
    expect((fragment.get(0) as Y.XmlElement).getAttribute('id')).toBe('blk_original')
  })
  it('leaves mount repairs and remote insertions out of history, and readers never repair', () => {
    const writer = setup(true, null)
    expect(ids(writer.editor).every(Boolean)).toBe(true)
    expect(writer.history.undoStack).toHaveLength(0)
    const reader = setup(false, null)
    expect(ids(reader.editor)).toEqual([null])
    expect(reader.history.undoStack).toHaveLength(0)
    const remote = new Y.Doc(); docs.push(remote)
    Y.applyUpdate(remote, Y.encodeStateAsUpdate(writer.doc))
    const paragraph = new Y.XmlElement('paragraph')
    paragraph.insert(0, [new Y.XmlText('Remote paragraph')])
    remote.getXmlFragment('prose').insert(1, [paragraph])
    Y.applyUpdate(writer.doc, Y.encodeStateAsUpdate(remote))
    expect(writer.editor.getText()).toContain('Remote paragraph')
    expect(ids(writer.editor).every(Boolean)).toBe(true)
    expect(writer.history.undoStack).toHaveLength(0)
  })
  it('restores a selected structural node after deletion and keeps redo valid', () => {
    const { editor, history } = setup()
    editor.commands.insertContentAt(editor.state.doc.content.size, { type: 'horizontalRule' })
    history.clear()
    const at = editor.state.doc.firstChild!.nodeSize
    editor.commands.setNodeSelection(at)
    const before = editor.getJSON()
    editor.commands.deleteSelection()
    const deleted = editor.getJSON()
    editor.commands.undo()
    expect(editor.getJSON()).toEqual(before)
    expect(editor.state.selection.toJSON()).toEqual({ type: 'node', anchor: at })
    editor.commands.redo()
    expect(editor.getJSON()).toEqual(deleted)
  })
  it('does not turn explicitly excluded insertions or ID repairs into undoable edits', () => {
    const { editor, history } = setup()
    const paragraph = editor.schema.nodes.paragraph!.create(null, editor.schema.text('Imported'))
    editor.view.dispatch(editor.state.tr.insert(editor.state.doc.content.size, paragraph).setMeta('addToHistory', false))
    expect(ids(editor).every(Boolean)).toBe(true)
    expect(history.undoStack).toHaveLength(0)
  })
})
