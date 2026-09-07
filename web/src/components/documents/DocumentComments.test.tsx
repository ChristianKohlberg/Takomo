import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import * as Y from 'yjs'
import { DocumentComments } from './DocumentComments'
import { DocumentCommentButton } from './DocumentCommentButton'
import { captureCommentAnchor, createCommentThread, readCommentThreads, resolveCommentThread } from '@/lib/document-comments'
import { DocumentCommentHighlight } from '@/lib/document-comment-highlight'

function setup() {
  const ydoc = new Y.Doc()
  const fragment = ydoc.getXmlFragment('prose')
  const p = new Y.XmlElement('paragraph'); p.insert(0, [new Y.XmlText('Selected text remains.')]); fragment.insert(0, [p])
  const editor = new Editor({ element: document.createElement('div'), extensions: [StarterKit.configure({ undoRedo: false }), Collaboration.configure({ fragment }), DocumentCommentHighlight.configure({ ydoc, sectionId: 'one' })] })
  editor.commands.setTextSelection({ from: 1, to: 14 })
  const draft = captureCommentAnchor(editor)!
  return { ydoc, editor, draft }
}
describe('DocumentComments', () => {
  it('filters document-wide threads and retains removed-section discussions for readers', () => {
    const { ydoc, editor, draft } = setup()
    const first = createCommentThread(ydoc, 'one', draft, 'Ada', 'First question')
    const second = createCommentThread(ydoc, 'two', draft, 'Bob', 'Second question')
    createCommentThread(ydoc, 'removed', draft, 'Bob', 'Retained question')
    resolveCommentThread(ydoc, second, true)
    const jump = vi.fn()
    const titles: Record<string, string> = { one: 'Billing', two: 'Reports' }
    const view = render(<DocumentComments ydoc={ydoc} editor={null} actor="Reader" canWrite={false} locale="en"
      sectionTitle={id => titles[id] ?? null} onShowThread={jump} onDraftConsumed={() => {}} onClose={() => {}} />)
    expect(screen.getByText('First question')).toBeTruthy()
    expect(screen.queryByText('Second question')).toBeNull()
    expect(screen.getByText('Section removed')).toBeTruthy()
    expect(screen.getAllByRole('button', { name: 'Go to text' })).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: 'Go to text' }))
    expect(jump).toHaveBeenCalledWith(expect.objectContaining({ id: first }))
    expect(screen.queryByRole('button', { name: 'Resolve' })).toBeNull()
    act(() => { resolveCommentThread(ydoc, first, true) })
    expect(screen.queryByText('First question')).toBeNull()
    fireEvent.change(screen.getByLabelText('Filter comments'), { target: { value: 'resolved' } })
    expect(screen.getByText('Second question')).toBeTruthy()
    expect(screen.getByText('First question')).toBeTruthy()
    expect(screen.queryByText('Retained question')).toBeNull()
    fireEvent.change(screen.getByLabelText('Filter comments'), { target: { value: 'all' } })
    expect(screen.getByText('Retained question')).toBeTruthy()
    view.unmount(); editor.destroy(); ydoc.destroy()
  })

  it('posts, replies, resolves and reopens while preserving quote and highlights', () => {
    const { ydoc, editor, draft } = setup()
    const view = render(<DocumentComments ydoc={ydoc} sectionId="one" editor={editor} actor="Ada" canWrite locale="en" draft={draft} onDraftConsumed={() => {}} onClose={() => {}} />)
    fireEvent.change(screen.getByLabelText('New comment'), { target: { value: 'Question about this.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Post comment' }))
    expect(readCommentThreads(ydoc)[0]?.messages[0]?.text).toBe('Question about this.')
    expect(editor.view.dom.querySelector('[data-comment-id]')?.textContent).toBe('Selected text')
    fireEvent.change(screen.getByRole('textbox', { name: 'Reply' }), { target: { value: 'Answered.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Reply' }))
    expect(readCommentThreads(ydoc)[0]?.messages).toHaveLength(2)
    fireEvent.click(screen.getByRole('button', { name: 'Resolve' }))
    expect(editor.view.dom.querySelector('[data-comment-id]')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Reopen' }))
    expect(editor.view.dom.querySelector('[data-comment-id]')).not.toBeNull()
    act(() => { editor.commands.deleteRange({ from: 1, to: 14 }) })
    expect(screen.getByText('Text changed or removed · quote retained')).toBeTruthy()
    view.unmount(); editor.destroy(); ydoc.destroy()
  })
  it('readers see discussion and orphan quote but cannot post, reply or resolve', () => {
    const { ydoc, editor, draft } = setup()
    createCommentThread(ydoc, 'one', draft, 'Ada', 'Visible to readers')
    const view = render(<DocumentComments ydoc={ydoc} sectionId="one" editor={editor} actor="Reader" canWrite={false} locale="en" draft={draft} onDraftConsumed={() => {}} onClose={() => {}} />)
    expect(screen.getByText('Visible to readers')).toBeTruthy()
    expect(screen.queryByRole('textbox')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Resolve' })).toBeNull()
    view.unmount(); editor.destroy(); ydoc.destroy()
  })
  it('enables comment creation for nonempty selection and captures its original quote', () => {
    const { ydoc, editor } = setup()
    let quote = ''
    const view = render(<DocumentCommentButton editor={editor} canWrite locale="en" onComment={anchor => { quote = anchor.quote }} />)
    fireEvent.click(screen.getByRole('button', { name: 'Add comment' }))
    expect(quote).toBe('Selected text')
    act(() => { editor.commands.setTextSelection(1) })
    expect((screen.getByRole('button', { name: 'Add comment' }) as HTMLButtonElement).disabled).toBe(true)
    view.unmount(); editor.destroy(); ydoc.destroy()
  })
})
