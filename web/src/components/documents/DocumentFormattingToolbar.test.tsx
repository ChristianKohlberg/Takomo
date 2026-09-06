import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { afterEach, describe, expect, it } from 'vitest'
import { DocumentFormattingToolbar } from './DocumentFormattingToolbar'
const editors: Editor[] = []
function mount(content = '<p>First paragraph</p><h2>Second heading</h2>') {
  const editor = new Editor({ extensions: [StarterKit], content })
  editors.push(editor)
  const view = render(<DocumentFormattingToolbar editor={editor} locale="en" canWrite />)
  return { editor, view, style: screen.getByRole('combobox', { name: 'Paragraph style' }) as HTMLSelectElement }
}
afterEach(() => { cleanup(); editors.splice(0).forEach(editor => editor.destroy()) })
describe('DocumentFormattingToolbar', () => {
  it('tracks the block and formats selected prose without losing text or selection', () => {
    const { editor, style } = mount()
    act(() => { editor.commands.setTextSelection({ from: 2, to: 8 }) })
    const text = editor.getText()
    expect(style.value).toBe('paragraph')
    fireEvent.focus(style)
    fireEvent.change(style, { target: { value: 'h3' } })
    expect(editor.state.doc.firstChild?.attrs.level).toBe(3)
    expect(editor.getText()).toBe(text)
    expect(editor.state.selection.from).toBe(2)
    expect(editor.state.selection.to).toBe(8)
    expect(style.value).toBe('h3')
    fireEvent.change(style, { target: { value: 'paragraph' } })
    expect(editor.state.doc.firstChild?.type.name).toBe('paragraph')
    act(() => { editor.commands.setTextSelection(19) })
    expect(style.value).toBe('h2')
  })
  it('shows mixed and unsupported styles honestly and applies one style across selected blocks', () => {
    const { editor, style } = mount('<p>First</p><h2>Second</h2><h4>Fourth</h4>')
    act(() => { editor.commands.setTextSelection({ from: 1, to: 12 }) })
    expect(style.value).toBe('mixed')
    fireEvent.change(style, { target: { value: 'h1' } })
    expect(editor.state.doc.child(0).attrs.level).toBe(1)
    expect(editor.state.doc.child(1).attrs.level).toBe(1)
    act(() => { editor.commands.setTextSelection(17) })
    expect(style.value).toBe('other')
  })
  it('applies marks and lists to the selection and reflects active state', () => {
    const { editor } = mount('<p>Selected words</p>')
    act(() => { editor.commands.setTextSelection({ from: 1, to: 9 }) })
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Bold' }))
    fireEvent.click(screen.getByRole('button', { name: 'Bold' }))
    expect(editor.state.doc.firstChild?.firstChild?.marks[0]?.type.name).toBe('bold')
    expect(screen.getByRole('button', { name: 'Bold' }).getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(screen.getByRole('button', { name: 'Bulleted list' }))
    expect(editor.state.doc.firstChild?.type.name).toBe('bulletList')
    expect(editor.state.doc.textContent).toBe('Selected words')
  })
  it('disables without an editor and when permission changes, and hides for readers', () => {
    const { editor, view, style } = mount()
    act(() => { editor.setEditable(false) })
    expect(style.disabled).toBe(true)
    expect((screen.getByRole('button', { name: 'Bold' }) as HTMLButtonElement).disabled).toBe(true)
    act(() => { editor.setEditable(true) })
    expect(style.disabled).toBe(false)
    view.rerender(<DocumentFormattingToolbar editor={null} locale="en" canWrite />)
    expect(style.disabled).toBe(true)
    view.rerender(<DocumentFormattingToolbar editor={editor} locale="en" canWrite={false} />)
    expect(screen.queryByRole('group', { name: 'Text formatting' })).toBeNull()
  })
})
