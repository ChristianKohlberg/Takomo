import { describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { captureCommentAnchor, createCommentThread } from './document-comments'
import { DocumentCommentHighlight } from './document-comment-highlight'

const settle = () => new Promise<void>(resolve => queueMicrotask(resolve))
const highlighted = (editor: Editor) => [...editor.view.dom.querySelectorAll('[data-comment-id]')].map(el => el.textContent)

function setup() {
  const ydoc = new Y.Doc()
  const fragment = ydoc.getXmlFragment('prose')
  const p = new Y.XmlElement('paragraph'); p.insert(0, [new Y.XmlText('Hello world again.')]); fragment.insert(0, [p])
  const editor = new Editor({ element: document.createElement('div'), extensions: [StarterKit.configure({ undoRedo: false }), Collaboration.configure({ fragment }), DocumentCommentHighlight.configure({ ydoc, sectionId: 'one' })] })
  editor.commands.setTextSelection({ from: 7, to: 12 })
  createCommentThread(ydoc, 'one', captureCommentAnchor(editor)!, 'Ada', 'Which world?')
  expect(highlighted(editor)).toEqual(['world'])
  return { ydoc, editor }
}
describe('comment highlight timing', () => {
  it('keeps the highlight on the quoted words through local typing before them, and after undo', async () => {
    const { ydoc, editor } = setup()
    editor.commands.insertContentAt(1, 'X')
    expect(highlighted(editor)).toEqual(['world'])
    editor.commands.insertContentAt(1, 'Y')
    expect(highlighted(editor)).toEqual(['world'])
    await settle()
    expect(highlighted(editor)).toEqual(['world'])
    expect(editor.state.doc.textContent).toBe('YXHello world again.')
    editor.commands.undo()
    expect(highlighted(editor)).toEqual(['world'])
    editor.destroy(); ydoc.destroy()
  })
  it('keeps the highlight when typing directly in front of and behind the quote', async () => {
    const { ydoc, editor } = setup()
    editor.commands.insertContentAt(7, 'big ')
    expect(highlighted(editor)).toEqual(['world'])
    await settle()
    expect(highlighted(editor)).toEqual(['world'])
    editor.commands.insertContentAt(16, 'wide')
    await settle()
    expect(highlighted(editor)).toEqual(['world'])
    expect(editor.state.doc.textContent).toBe('Hello big worldwide again.')
    editor.destroy(); ydoc.destroy()
  })
  it('detaches on deletion, stays detached from a retyped duplicate, and follows a remote edit', async () => {
    const { ydoc, editor } = setup()
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(ydoc))
    const text = peer.getXmlFragment('prose').get(0) as Y.XmlElement
    ;(text.get(0) as Y.XmlText).insert(0, 'Remote ')
    Y.applyUpdate(ydoc, Y.encodeStateAsUpdate(peer))
    expect(editor.state.doc.textContent).toBe('Remote Hello world again.')
    expect(highlighted(editor)).toEqual(['world'])
    editor.commands.deleteRange({ from: 14, to: 19 })
    expect(highlighted(editor)).toEqual([])
    editor.commands.insertContentAt(14, 'world')
    expect(editor.state.doc.textContent).toBe('Remote Hello world again.')
    await settle()
    expect(highlighted(editor)).toEqual([])
    editor.destroy(); ydoc.destroy(); peer.destroy()
  })
  it('drops a highlight typed over after the sync settles, but keeps an identical replacement', async () => {
    const { ydoc, editor } = setup()
    editor.commands.insertContentAt({ from: 7, to: 12 }, 'world')
    await settle()
    expect(highlighted(editor)).toEqual(['world'])
    editor.commands.insertContentAt({ from: 7, to: 12 }, 'earth')
    await settle()
    expect(highlighted(editor)).toEqual([])
    editor.destroy(); ydoc.destroy()
  })
})
