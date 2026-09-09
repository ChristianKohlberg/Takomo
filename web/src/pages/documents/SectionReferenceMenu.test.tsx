import { act, fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import type { Editor } from '@tiptap/react'
import * as Y from 'yjs'
import { createNode, proseOf } from '@/lib/mindmap-crdt'
import { createStructureHistory } from '@/lib/plan-structure'
import { referenceMatch } from '@/lib/section-reference-trigger'
import SectionEditor from './SectionEditor'
it('routes native picker keys before section navigation and undoes a shared CRDT insertion as one action', () => {
  const doc = new Y.Doc()
  const source = createNode(doc, { title: 'Source', parent: null, by: 'test' })!
  const target = createNode(doc, { title: 'Billing', parent: null, by: 'test' })!
  const history = createStructureHistory(doc)
  const provider = { awareness: new Awareness(doc) } as unknown as WebsocketProvider
  let editor!: Editor
  const mounted = render(<SectionEditor ydoc={doc} sectionId={source} fragment={proseOf(doc, source)!} provider={provider} history={history}
    onSettled={() => {}} display="Ada" color="#123456" canWrite label="Source" onEditor={value => { if (value) editor = value }} />)
  vi.spyOn(editor.view, 'coordsAtPos').mockReturnValue({ left: 10, right: 10, top: 10, bottom: 20 })
  act(() => { editor.commands.insertContent('See ') })
  act(() => { const { from, to } = editor.state.selection; editor.view.someProp('handleTextInput', fn => fn(editor.view, from, to, '@', () => editor.state.tr.insertText('@'))) })
  expect(screen.getByRole('listbox', { name: 'Sections' })).toBeTruthy()
  fireEvent.keyDown(editor.view.dom, { key: 'ArrowDown' })
  fireEvent.keyDown(editor.view.dom, { key: 'Enter' })
  expect(editor.getText()).toBe('See 2 Billing')
  expect(editor.state.doc.firstChild!.child(1).attrs.sectionId).toBe(target)
  act(() => { history.undo() })
  expect(editor.getText()).toBe('See @')
  act(() => { history.redo() })
  expect(editor.getText()).toBe('See 2 Billing')
  act(() => { editor.commands.insertContent(' '); const { from, to } = editor.state.selection; editor.view.someProp('handleTextInput', fn => fn(editor.view, from, to, '@', () => editor.state.tr.insertText('@'))) })
  expect(referenceMatch(editor.state)).not.toBeNull()
  const peer = new Y.Doc(); Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
  const paragraph = proseOf(peer, source)!.get(0) as Y.XmlElement
  const text = paragraph.get(paragraph.length - 1) as Y.XmlText
  text.delete(text.length - 1, 1)
  act(() => { Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer)) })
  expect(referenceMatch(editor.state)).toBeNull()
  expect(screen.queryByRole('listbox', { name: 'Sections' })).toBeNull()
  mounted.unmount(); history.destroy(); provider.awareness.destroy(); peer.destroy(); doc.destroy()
})
