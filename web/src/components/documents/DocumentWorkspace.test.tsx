import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentAgent } from './DocumentAgent'
import { DocumentConversation } from './DocumentConversation'
import { getDocumentConversation, sendDocumentMessage, setDocumentPins, type DocumentConversationView } from '@/lib/document-conversation'

vi.mock('@/lib/document-conversation', () => ({ getDocumentConversation: vi.fn(), sendDocumentMessage: vi.fn(), setDocumentPins: vi.fn() }))
const get = vi.mocked(getDocumentConversation)
const post = vi.mocked(sendDocumentMessage)
const patch = vi.mocked(setDocumentPins)
const empty: DocumentConversationView = { conversation: null, messages: [], jobs: [], pinned_section_ids: [] }
const props = { token: 'token', project: 'project', map: 'map', lang: 'en' as const, canAsk: true, selected: 'one', onError: vi.fn(), nodes: [{ id: 'one', title: 'Requirements', parent: null, position: 0, order: 'a' }, { id: 'two', title: 'Errors', parent: 'one', position: 0, order: 'a' }] }
const conversation = { ...props, open: true, onOpenChange: vi.fn(), restoreFocus: vi.fn(), intent: { action: 'discuss' as const, mode: 'automatic' as const, whole_document: false, section_ids: [], nonce: 0 } }
beforeEach(() => { vi.clearAllMocks(); get.mockResolvedValue(empty); post.mockResolvedValue(empty); patch.mockResolvedValue(empty) })

function workspace() {
  return render(<DocumentAgent {...props}>{tools => <><div>{tools}</div><div data-section="one" contentEditable suppressContentEditableWarning tabIndex={0}>The user can sign in.</div></>}</DocumentAgent>)
}
async function open() { fireEvent.click(screen.getByRole('button', { name: 'Discuss with Codex' })); return screen.findByRole('textbox') }

describe('document conversation workspace', () => {
  it('docks without a focus trap, preserves the mounted editor and draft across mobile tabs, and resizes by keyboard', async () => {
    workspace()
    const editor = screen.getByText('The user can sign in.')
    const prompt = await open()
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.change(prompt, { target: { value: 'An unfinished thought' } })
    const separator = screen.getByRole('separator')
    fireEvent.keyDown(separator, { key: 'ArrowLeft' })
    expect(separator.getAttribute('aria-valuenow')).toBe('444')
    fireEvent.click(screen.getByRole('tab', { name: 'Document' }))
    editor.focus()
    expect(document.activeElement).toBe(editor)
    expect(editor.isConnected).toBe(true)
    fireEvent.click(screen.getByRole('tab', { name: 'Chat' }))
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('An unfinished thought')
  })

  it('starts empty in automatic mode and recognizes a slash command with custom instructions', async () => {
    render(<DocumentConversation {...conversation} />)
    const prompt = await screen.findByRole('textbox')
    expect((prompt as HTMLTextAreaElement).value).toBe('')
    expect((screen.getByRole('radio', { name: 'Automatic' }) as HTMLInputElement).checked).toBe(true)
    fireEvent.change(prompt, { target: { value: '/grill focus on permissions' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(post).toHaveBeenCalled())
    expect(post.mock.calls[0]![2]).toMatchObject({ action: 'grill', message: 'focus on permissions', context: { mode: 'automatic', section_ids: [], pinned_section_ids: [] } })
  })

  it('offers described slash actions and consumes Escape before closing the conversation', async () => {
    render(<DocumentConversation {...conversation} />)
    const prompt = await screen.findByRole('textbox')
    fireEvent.change(prompt, { target: { value: '/' } })
    expect(screen.getAllByRole('option').filter(option => option.tagName === 'BUTTON')).toHaveLength(3)
    expect(screen.getByRole('listbox').textContent).toContain('one focused question at a time')
    fireEvent.keyDown(prompt, { key: 'ArrowDown' })
    fireEvent.keyDown(prompt, { key: 'Enter' })
    expect((prompt as HTMLTextAreaElement).value).toContain('acceptance tests')
    expect(post).not.toHaveBeenCalled()
    fireEvent.change(prompt, { target: { value: '/' } })
    fireEvent.keyDown(prompt, { key: 'Escape' })
    expect(screen.queryByRole('listbox')).toBeNull()
    expect(conversation.onOpenChange).not.toHaveBeenCalled()
  })

  it('captures a same-section text selection without editing the document and clears the quote on automatic scope', async () => {
    workspace()
    const editor = screen.getByText('The user can sign in.')
    const range = document.createRange(); range.selectNodeContents(editor)
    const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range)
    fireEvent(document, new Event('selectionchange'))
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Discuss in chat' }))
    fireEvent.click(screen.getByRole('button', { name: 'Discuss in chat' }))
    const prompt = await screen.findByRole('textbox')
    expect(editor.textContent).toBe('The user can sign in.')
    expect(screen.getByText('Quoted text')).toBeTruthy()
    fireEvent.change(prompt, { target: { value: 'Does this need clarification?' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(post).toHaveBeenCalled())
    expect(post.mock.calls[0]![2].context).toMatchObject({ mode: 'selected', section_ids: ['one'], quote: { section_id: 'one', text: 'The user can sign in.' } })
    selection.removeAllRanges()
  })

  it('clears quoted context when explicitly switching to automatic mode', async () => {
    render(<DocumentConversation {...conversation} intent={{ ...conversation.intent, mode: 'selected', section_ids: ['one'], quote: { section_id: 'one', text: 'The user can sign in.' } }} />)
    const prompt = await screen.findByRole('textbox')
    expect(screen.getByText('Quoted text')).toBeTruthy()
    fireEvent.click(screen.getByRole('radio', { name: 'Automatic' }))
    expect(screen.queryByText('Quoted text')).toBeNull()
    fireEvent.change(prompt, { target: { value: 'Find the remaining gaps' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(post).toHaveBeenCalled())
    expect(post.mock.calls[0]![2].context.quote).toBeUndefined()
  })

  it('persists pins independently of sending and restores them on remount', async () => {
    get.mockResolvedValue({ ...empty, pinned_section_ids: ['two'] })
    const ui = render(<DocumentConversation {...conversation} />)
    await screen.findByRole('textbox')
    fireEvent.click(screen.getByText('Pinned sections (1)'))
    const pins = screen.getByText('Pinned sections (1)').closest('details')!
    expect((within(pins).getByRole('checkbox', { name: /Errors/ }) as HTMLInputElement).checked).toBe(true)
    get.mockResolvedValue({ ...empty, pinned_section_ids: ['two', 'one'] })
    fireEvent.click(within(pins).getByRole('checkbox', { name: /Requirements/ }))
    await waitFor(() => expect(patch).toHaveBeenCalledWith('token', 'map', ['two', 'one'], expect.any(AbortSignal)))
    expect(post).not.toHaveBeenCalled()
    ui.unmount()
    render(<DocumentConversation {...conversation} />)
    await screen.findByText('Pinned sections (2)')
  })

  it('renders only evidence-backed citations as navigation and distinguishes source snippets from full reads', async () => {
    const history: DocumentConversationView = { ...empty, conversation: { id: 'chat' }, messages: [{ id: 'answer', role: 'assistant', body: 'See [Requirements](takomo-section:one) and [untrusted](takomo-section:invented).', created_at: 2, job_id: 'job' }], jobs: [{ id: 'job', status: 'completed', action: 'discuss', section_ids: [], whole_document: false, section_count: 2, error: null, created_at: 1, sources: [{ section_id: 'one', title: 'Original requirement', version: 'abc123def456' }], coverage: { read_section_ids: [], total_sections: 2, complete: false } }] }
    get.mockResolvedValue(history)
    const navigate = vi.fn()
    render(<DocumentConversation {...conversation} onNavigate={navigate} canAsk={false} />)
    fireEvent.click(await screen.findByRole('link', { name: 'Requirements' }))
    expect(navigate).toHaveBeenCalledWith('one')
    expect(screen.queryByRole('link', { name: 'untrusted' })).toBeNull()
    expect(screen.getByText(/0\/2 sections fully read/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Original requirement' }))
    expect(navigate).toHaveBeenCalledTimes(2)
    expect(screen.queryByRole('textbox')).toBeNull()
  })

  it('shows migration context above pending or completed replies without exposing thread IDs', async () => {
    const job = { id: 'job', status: 'running' as const, action: 'discuss' as const, section_ids: [], whole_document: false, section_count: 2, error: null, created_at: 1, migration: { previous_thread_id: 'private-old-thread', new_thread_id: 'private-new-thread', retained_turns: 8, omitted_turns: 3 } }
    get.mockResolvedValue({ ...empty, jobs: [job] })
    const ui = render(<DocumentConversation {...conversation} />)
    await screen.findByText(/8 earlier turns were carried forward/)
    expect(screen.getByText(/3 older turns were left out/)).toBeTruthy()
    expect(document.body.textContent).not.toContain('private-old-thread')
    expect(document.body.textContent).not.toContain('private-new-thread')
    ui.unmount()
    get.mockResolvedValue({ ...empty, jobs: [{ ...job, status: 'completed', migration: { ...job.migration, omitted_turns: 0 } }], messages: [{ id: 'answer', job_id: 'job', role: 'assistant', body: 'Continuing the discussion.', created_at: 2 }] })
    render(<DocumentConversation {...conversation} />)
    await screen.findByText('Continuing the discussion.')
    expect(screen.getAllByText(/8 earlier turns were carried forward/)).toHaveLength(1)
    expect(screen.queryByText(/older turns were left out/)).toBeNull()
  })

  it('ignores late pin updates after a document switch', async () => {
    let resolve!: (value: DocumentConversationView) => void
    patch.mockReturnValue(new Promise(done => { resolve = done }))
    const ui = render(<DocumentConversation {...conversation} />)
    await screen.findByRole('textbox')
    fireEvent.click(screen.getByText('Pinned sections (0)'))
    fireEvent.click(within(screen.getByText('Pinned sections (0)').closest('details')!).getByRole('checkbox', { name: /Requirements/ }))
    ui.rerender(<DocumentConversation {...conversation} map="other" />)
    await act(async () => resolve({ ...empty, pinned_section_ids: ['one'] }))
    expect(patch.mock.calls[0]![3]?.aborted).toBe(true)
    expect(await screen.findByText('Pinned sections (0)')).toBeTruthy()
  })
})
