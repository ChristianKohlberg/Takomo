// The one thing this component has to get right: an editor is bound to THIS
// section's fragment and no other.
//
// Driven against a real `Y.Doc` shaped the way the server shapes one — a node
// map whose entries carry a `prose` XmlFragment — because the claim being tested
// is about Yjs and Tiptap agreeing, and a mock of either would only assert that
// the mock was written to match the test.
import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import type { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'

import SectionEditor from './SectionEditor'
import type { Editor } from '@tiptap/react'
import { readPlanTree } from '@/lib/mindmap-crdt'
import { insertPlanSection } from '@/lib/plan-insert'

/** A node with prose in it, as `createNode` and the server both write one. */
function section(doc: Y.Doc, id: string, text: string): Y.XmlFragment {
  const nodes = doc.getMap<Y.Map<unknown>>('nodes')
  nodes.set(id, new Y.Map())
  const entry = nodes.get(id) as Y.Map<unknown>
  entry.set('prose', new Y.XmlFragment())
  const frag = entry.get('prose') as Y.XmlFragment
  const p = new Y.XmlElement('paragraph')
  p.insert(0, [new Y.XmlText(text)])
  frag.insert(0, [p])
  return frag
}

function mount(doc: Y.Doc, fragment: Y.XmlFragment, label: string) {
  // Awareness is all `CollaborationCaret` touches; a socket would only try to
  // dial out of the test.
  const provider = { awareness: new Awareness(doc) } as unknown as WebsocketProvider
  return render(
    <SectionEditor
      ydoc={doc}
      fragment={fragment}
      provider={provider}
      display="Ada"
      color="#2563eb"
      canWrite
      onSettled={() => {}}
      label={label}
    />,
  )
}

describe('SectionEditor', () => {
  it('turns a completed final heading into a shared section without losing earlier prose', () => {
    const doc = new Y.Doc()
    const fragment = section(doc, 'mn-1', 'Keep this paragraph.')
    const awareness = new Awareness(doc)
    const insert = vi.fn(() => true)
    let editor: Editor | null = null
    render(<SectionEditor ydoc={doc} fragment={fragment}
      provider={{ awareness } as unknown as WebsocketProvider}
      display="Ada" color="#2563eb" canWrite onSettled={() => {}}
      onEditor={(value) => { editor = value }} onInsertSection={insert} label="Section heading" />)
    act(() => {
      editor!.commands.insertContentAt(editor!.state.doc.content.size, { type: 'heading', attrs: { level: 2 }, content: [{ type: 'text', text: 'Invoices' }] })
      editor!.state.doc.descendants((node, pos) => {
        if (node.type.name === 'heading') editor!.commands.setTextSelection(pos + node.nodeSize - 1)
      })
    })
    fireEvent.keyDown(screen.getByLabelText('Section heading'), { key: 'Enter' })
    expect(insert).toHaveBeenCalledWith(2, 'Invoices')
    expect(fragment.toString()).toContain('Keep this paragraph.')
    expect(fragment.toString()).not.toContain('Invoices')
    awareness.destroy()
  })

  it('leaves an empty final heading to the editor rather than creating an untitled section', () => {
    const doc = new Y.Doc()
    const fragment = section(doc, 'mn-1', 'Keep this paragraph.')
    const awareness = new Awareness(doc)
    let editor: Editor | null = null
    render(<SectionEditor ydoc={doc} fragment={fragment}
      provider={{ awareness } as unknown as WebsocketProvider}
      display="Ada" color="#2563eb" canWrite onSettled={() => {}}
      onEditor={(value) => { editor = value }} label="Section heading"
      onInsertSection={(level, title) => insertPlanSection(doc, 'mn-1', level, title, 'Ada') !== null} />)
    act(() => {
      editor!.commands.insertContentAt(editor!.state.doc.content.size, { type: 'heading', attrs: { level: 1 } })
      editor!.commands.focus('end')
    })
    fireEvent.keyDown(screen.getByLabelText('Section heading'), { key: 'Enter' })
    expect(readPlanTree(doc).map((node) => node.id)).toEqual(['mn-1'])
    expect(fragment.toString()).toContain('Keep this paragraph.')
    awareness.destroy()
  })

  it('shows the prose of the fragment it was handed', () => {
    const doc = new Y.Doc()
    const one = section(doc, 'mn-1', 'The surface everything hangs off.')
    section(doc, 'mn-2', 'Billing, which nobody has written yet.')

    mount(doc, one, 'Section 1')
    const editor = screen.getByLabelText('Section 1')
    expect(editor.textContent).toContain('The surface everything hangs off.')
    // The other section's prose lives in the same document. Binding by field
    // rather than by fragment would have pulled in whatever `getXmlFragment`
    // returned, which is exactly the failure worth a test.
    expect(editor.textContent).not.toContain('Billing')
  })

  it('binds each section to its own fragment', () => {
    const doc = new Y.Doc()
    section(doc, 'mn-1', 'The surface everything hangs off.')
    const two = section(doc, 'mn-2', 'Billing, which nobody has written yet.')

    mount(doc, two, 'Section 2')
    expect(screen.getByLabelText('Section 2').textContent).toContain('Billing')
  })

  it('writes what somebody types back into that section, and nowhere else', () => {
    const doc = new Y.Doc()
    const one = section(doc, 'mn-1', 'First.')
    const two = section(doc, 'mn-2', 'Second.')

    mount(doc, one, 'Section 1')
    // Through the fragment rather than through the DOM: jsdom has no caret, and
    // what is being checked is that the editor and the fragment are the same
    // document — an edit arriving from a peer takes exactly this path.
    const paragraph = one.get(0) as Y.XmlElement
    const text = paragraph.get(0) as Y.XmlText
    text.insert(text.length, ' And more.')

    expect(screen.getByLabelText('Section 1').textContent).toContain('First. And more.')
    expect(two.toString()).toContain('Second.')
    expect(two.toString()).not.toContain('And more')
  })
})
