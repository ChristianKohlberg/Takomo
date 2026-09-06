import { describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { captureCommentAnchor, createCommentThread, readCommentThreads, replyToComment, resolveCommentAnchor, resolveCommentThread } from './document-comments'

function setup() {
  const doc = new Y.Doc()
  const fragment = doc.getXmlFragment('prose')
  const p = new Y.XmlElement('paragraph')
  p.insert(0, [new Y.XmlText('Before selected words after selected words.')])
  fragment.insert(0, [p])
  const editor = new Editor({ element: document.createElement('div'), extensions: [StarterKit.configure({ undoRedo: false }), Collaboration.configure({ fragment })] })
  editor.commands.setTextSelection({ from: 8, to: 22 })
  return { doc, editor }
}
describe('selection comments', () => {
  it('tracks edits outside the range without finding a duplicate quote', () => {
    const { doc, editor } = setup()
    const anchor = captureCommentAnchor(editor)!
    expect(anchor.quote).toBe('selected words')
    editor.commands.insertContentAt(1, 'New ')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 12, to: 26 })
    editor.commands.deleteRange({ from: 12, to: 26 })
    expect(resolveCommentAnchor(editor, anchor)).toBeNull()
    expect(anchor.quote).toBe('selected words')
    editor.destroy(); doc.destroy()
  })
  it('stays attached when text is typed at either exact boundary, locally, remotely and through undo', () => {
    const { doc, editor } = setup()
    const anchor = captureCommentAnchor(editor)!
    editor.commands.insertContentAt(8, 'big ')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 12, to: 26 })
    editor.commands.insertContentAt(26, ' indeed')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 12, to: 26 })
    editor.commands.undo()
    editor.commands.undo()
    expect(editor.state.doc.textContent).toBe('Before selected words after selected words.')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 8, to: 22 })
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    const text = (peer.getXmlFragment('prose').get(0) as Y.XmlElement).get(0) as Y.XmlText
    text.insert(21, ' truly')
    text.insert(7, 'very ')
    Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer))
    expect(editor.state.doc.textContent).toBe('Before very selected words truly after selected words.')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 13, to: 27 })
    editor.destroy(); doc.destroy(); peer.destroy()
  })
  it('keeps a quote at the start of the first paragraph attached when text is typed in front of it', () => {
    const { doc, editor } = setup()
    editor.commands.setTextSelection({ from: 1, to: 7 })
    const anchor = captureCommentAnchor(editor)!
    expect(anchor.quote).toBe('Before')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 1, to: 7 })
    editor.commands.insertContentAt(1, 'Well ')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 6, to: 12 })
    editor.commands.insertContentAt(12, '!')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 6, to: 12 })
    editor.commands.deleteRange({ from: 6, to: 12 })
    editor.commands.insertContentAt(6, 'Before')
    expect(editor.state.doc.textContent).toBe('Well Before! selected words after selected words.')
    expect(resolveCommentAnchor(editor, anchor)).toBeNull()
    editor.destroy(); doc.destroy()
  })
  it('does not attach to replacement text even when a matching passage remains', () => {
    const { doc, editor } = setup()
    const anchor = captureCommentAnchor(editor)!
    // Tiptap's Yjs binding optimizes identical replacement into a CRDT no-op.
    editor.commands.insertContentAt({ from: 8, to: 22 }, 'selected words')
    expect(resolveCommentAnchor(editor, anchor)).toEqual({ from: 8, to: 22 })
    // Actual deletion and later retyping create new characters; those must not
    // inherit a discussion belonging to the deleted characters.
    editor.commands.deleteRange({ from: 8, to: 22 })
    editor.commands.insertContentAt(8, 'selected words')
    expect(resolveCommentAnchor(editor, anchor)).toBeNull()
    editor.destroy(); doc.destroy()
  })
  it('persists threads and merges independent concurrent replies', () => {
    const { doc, editor } = setup()
    const id = createCommentThread(doc, 'section', captureCommentAnchor(editor)!, 'Ada', 'Check this')
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    replyToComment(doc, id, 'Ada', 'First reply')
    replyToComment(peer, id, 'Ben', 'Concurrent reply')
    Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer))
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    expect(readCommentThreads(doc, 'section')[0]?.messages).toHaveLength(3)
    expect(readCommentThreads(peer)).toEqual(readCommentThreads(doc))
    resolveCommentThread(doc, id, true)
    expect(readCommentThreads(doc)[0]?.resolved).toBe(true)
    resolveCommentThread(doc, id, false)
    expect(readCommentThreads(doc)[0]?.resolved).toBe(false)
    expect(readCommentThreads(doc, 'other')).toEqual([])
    editor.destroy(); doc.destroy(); peer.destroy()
  })
  it('restores anchors from a durable binary snapshot and follows a peer edit', () => {
    const { doc, editor } = setup()
    createCommentThread(doc, 'section', captureCommentAnchor(editor)!, 'Ada', 'Persist me')
    const restored = new Y.Doc()
    Y.applyUpdate(restored, Y.encodeStateAsUpdate(doc))
    const restoredEditor = new Editor({ element: document.createElement('div'), extensions: [StarterKit.configure({ undoRedo: false }), Collaboration.configure({ fragment: restored.getXmlFragment('prose') })] })
    const anchor = readCommentThreads(restored)[0]!.anchor
    expect(resolveCommentAnchor(restoredEditor, anchor)).toEqual({ from: 8, to: 22 })
    editor.commands.insertContentAt(1, 'Peer ')
    Y.applyUpdate(restored, Y.encodeStateAsUpdate(doc))
    expect(resolveCommentAnchor(restoredEditor, anchor)).toEqual({ from: 13, to: 27 })
    editor.destroy(); restoredEditor.destroy(); doc.destroy(); restored.destroy()
  })
  it('refuses empty selections and empty messages', () => {
    const { doc, editor } = setup()
    editor.commands.setTextSelection(1)
    expect(captureCommentAnchor(editor)).toBeNull()
    expect(() => replyToComment(doc, 'missing', 'Ada', ' ')).toThrow()
    editor.destroy(); doc.destroy()
  })
})
