import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import * as Y from 'yjs'
import { createNode } from '@/lib/mindmap-crdt'
import { DocumentSectionReference } from '@/lib/document-section-reference'
import { DocumentSectionReferenceButton } from './DocumentSectionReferenceButton'

describe('section reference picker', () => {
  it('searches headings and inserts at the existing prose selection', () => {
    const doc = new Y.Doc()
    const id = createNode(doc, { title: 'Billing rules', parent: null, by: 'test' })
    createNode(doc, { title: 'Accounts', parent: null, by: 'test' })
    const editor = new Editor({ extensions: [StarterKit, DocumentSectionReference.configure({ ydoc: doc })], content: '<p>Before after</p>' })
    editor.commands.setTextSelection(8)
    const view = render(<DocumentSectionReferenceButton editor={editor} ydoc={doc} canWrite locale="en" />)
    fireEvent.click(screen.getByLabelText('Insert section reference'))
    fireEvent.change(screen.getByLabelText('Search sections'), { target: { value: 'bill' } })
    expect(screen.queryByRole('button', { name: /Accounts/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /Billing rules/ }))
    const content = editor.state.doc.firstChild!
    expect(content.child(0).text).toBe('Before ')
    expect(content.child(1).attrs.sectionId).toBe(id)
    expect(content.child(2).text).toBe('after')
    view.unmount(); editor.destroy(); doc.destroy()
  })
  it('disables insertion without an editor and hides it for readers', () => {
    const doc = new Y.Doc()
    const view = render(<DocumentSectionReferenceButton editor={null} ydoc={doc} canWrite locale="en" />)
    expect((screen.getByLabelText('Insert section reference') as HTMLButtonElement).disabled).toBe(true)
    act(() => view.rerender(<DocumentSectionReferenceButton editor={null} ydoc={doc} canWrite={false} locale="en" />))
    expect(screen.queryByLabelText('Insert section reference')).toBeNull()
    view.unmount(); doc.destroy()
  })
})
