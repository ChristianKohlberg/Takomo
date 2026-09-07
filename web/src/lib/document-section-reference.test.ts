import { afterEach, describe, expect, it, vi } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { DocumentSectionReference } from './document-section-reference'
import { createNode, nodesMap, proseOf, proseTextOf, setTitle } from './mindmap-crdt'

const editors: Editor[] = []
const docs: Y.Doc[] = []
afterEach(() => { editors.splice(0).forEach(e => e.destroy()); docs.splice(0).forEach(d => d.destroy()) })
function setup(onNavigate: ((id: string) => void) | null = null) {
  const doc = new Y.Doc(); docs.push(doc)
  const id = createNode(doc, { title: 'Original heading', parent: null, by: 'test' })!
  const source = createNode(doc, { title: 'Source', parent: null, by: 'test' })!
  const fragment = proseOf(doc, source)!
  const editor = new Editor({ element: document.createElement('div'), extensions: [StarterKit.configure({ undoRedo: false }),
    Collaboration.configure({ document: doc, fragment }), DocumentSectionReference.configure({ ydoc: doc, project: () => 'demo', onNavigate })] })
  editors.push(editor)
  editor.commands.insertContent({ type: 'sectionReference', attrs: { sectionId: id }, content: [{ type: 'text', text: 'Original heading' }] })
  return { doc, id, source, editor, fragment }
}
describe('document section references', () => {
  it('uses in-document navigation for ordinary clicks and preserves modified links', () => {
    const navigate = vi.fn()
    const { editor, id } = setup(navigate)
    const link = editor.view.dom.querySelector('a')!
    const ordinary = new MouseEvent('click', { button: 0, cancelable: true })
    link.dispatchEvent(ordinary)
    expect(navigate).toHaveBeenCalledWith(id)
    expect(ordinary.defaultPrevented).toBe(true)
    const modified = new MouseEvent('click', { button: 0, ctrlKey: true, cancelable: true })
    link.dispatchEvent(modified)
    expect(navigate).toHaveBeenCalledTimes(1)
    expect(modified.defaultPrevented).toBe(false)
  })
  it('follows renames without modifying shared prose and keeps stable links', () => {
    const { doc, id, editor, fragment } = setup()
    const before = fragment.toString()
    setTitle(doc, id, 'Renamed heading')
    expect(editor.view.dom.querySelector('a')?.textContent).toBe('Renamed heading')
    expect(editor.view.dom.querySelector('a')?.getAttribute('href')).toContain(`section=${id}`)
    expect(editor.getText()).toBe('Renamed heading')
    expect(fragment.toString()).toBe(before)
    expect(editor.getHTML()).toContain('Renamed heading')
  })
  it('retains plain text for generic CRDT readers and disables deleted references without losing the label', () => {
    const { doc, id, source, editor } = setup()
    expect(proseTextOf(doc, source)).toBe('Original heading')
    nodesMap(doc).delete(id)
    expect(editor.view.dom.querySelector('a')?.textContent).toBe('Original heading (Missing section)')
    expect(editor.view.dom.querySelector('a')?.hasAttribute('href')).toBe(false)
    expect(proseTextOf(doc, source)).toBe('Original heading')
  })
  it('survives a CRDT reload and follows a remote rename without changing the prose', () => {
    const { doc, id, source } = setup()
    const replica = new Y.Doc(); docs.push(replica)
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(doc))
    const fragment = proseOf(replica, source)!
    const editor = new Editor({ extensions: [StarterKit.configure({ undoRedo: false }),
      Collaboration.configure({ document: replica, fragment }), DocumentSectionReference.configure({ ydoc: replica, project: () => 'demo' })] })
    editors.push(editor)
    expect(proseTextOf(replica, source)).toBe('Original heading')
    const before = fragment.toString()
    setTitle(doc, id, 'Remote renamed')
    Y.applyUpdate(replica, Y.encodeStateAsUpdate(doc))
    expect(editor.view.dom.querySelector('a')?.textContent).toBe('Remote renamed')
    expect(fragment.toString()).toBe(before)
  })
  it('preserves same-project reference identity through HTML paste', () => {
    const { editor, doc, id } = setup()
    const other = new Editor({ extensions: [StarterKit, DocumentSectionReference.configure({ ydoc: doc, project: () => 'demo' })], content: editor.getHTML() })
    editors.push(other)
    expect(other.state.doc.firstChild?.firstChild?.attrs.sectionId).toBe(id)
    expect(other.view.dom.querySelector('a')?.textContent).toBe('Original heading')
  })
  it('pastes a foreign-project reference as an ordinary link to its original destination', () => {
    const { editor, id } = setup()
    const doc = new Y.Doc(); docs.push(doc)
    const other = new Editor({ extensions: [StarterKit, DocumentSectionReference.configure({ ydoc: doc, project: () => 'elsewhere' })], content: editor.getHTML() })
    editors.push(other)
    const text = other.getJSON().content?.[0]?.content?.[0]
    expect(text?.type).toBe('text')
    expect(text?.marks?.[0]?.attrs?.href).toBe(`/projects/demo/specification?view=document&section=${id}`)
    expect(other.getText()).toBe('Original heading')
  })
})
