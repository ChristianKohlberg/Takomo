import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentConversation } from './DocumentConversation'
import { getDocumentConversation, sendDocumentMessage, type DocumentConversationView } from '@/lib/document-conversation'

vi.mock('@/lib/document-conversation', () => ({ getDocumentConversation: vi.fn(), sendDocumentMessage: vi.fn(), setDocumentPins: vi.fn() }))
const get = vi.mocked(getDocumentConversation)
const post = vi.mocked(sendDocumentMessage)
const empty: DocumentConversationView = { conversation: null, messages: [], jobs: [] }
const completed: DocumentConversationView = {
  conversation: { id: 'chat' },
  messages: [{ id: 'm1', job_id: 'job', role: 'user', body: 'Review these requirements', created_at: 1 }, { id: 'm2', job_id: 'job', role: 'assistant', body: 'A reviewable test draft.', created_at: 2 }],
  jobs: [{ id: 'job', action: 'draft_tests', section_ids: ['one', 'two'], whole_document: false, section_count: 2, sections: [{ id: 'one', title: 'Original title' }, { id: 'two', title: 'Second' }], status: 'completed', error: null, created_at: 1 }],
}
const props = {
  token: 'token', project: 'project', map: 'map', lang: 'en' as const, canAsk: true, open: true,
  onOpenChange: vi.fn(), restoreFocus: vi.fn(), onError: vi.fn(),
  nodes: [{ id: 'one', title: 'First', parent: null, position: 0, order: 'a' }, { id: 'two', title: 'Second', parent: null, position: 1, order: 'b' }],
  intent: { action: 'grill' as const, section_ids: ['one'], whole_document: false, nonce: 0 },
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done }); return { promise, resolve } }
beforeEach(() => { vi.clearAllMocks(); get.mockResolvedValue(empty); post.mockResolvedValue(completed) })

describe('document discussion', () => {
  it('sends exactly the checked sections and editable action prompt without auto-sending', async () => {
    render(<DocumentConversation {...props} />)
    await screen.findByRole('checkbox', { name: 'First' })
    expect(post).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('checkbox', { name: 'Second' }))
    fireEvent.change(screen.getByRole('combobox', { name: 'Action' }), { target: { value: 'draft_tests' } })
    fireEvent.change(screen.getByRole('textbox', { name: 'Your instruction or reply' }), { target: { value: 'Test the failure paths too' } })
    get.mockResolvedValue(completed)
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await screen.findByText('A reviewable test draft.')
    expect(post).toHaveBeenCalledWith('token', 'map', { message: 'Test the failure paths too', request_id: expect.any(String), action: 'draft_tests', context: { mode: 'selected', section_ids: ['one', 'two'], pinned_section_ids: [] } }, expect.any(AbortSignal))
    expect(within(screen.getByText('Review these requirements').closest('article')!).getByText('Original title')).toBeTruthy()
  })

  it('requires explicit whole-document selection and blocks missing section scope', async () => {
    render(<DocumentConversation {...props} intent={{ ...props.intent, section_ids: [] }} />)
    await screen.findByRole('checkbox', { name: 'First' })
    expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('radio', { name: 'Whole document' }))
    expect(screen.getByText('Whole document · 2 sections')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(post).toHaveBeenCalled())
    expect(post.mock.calls[0]![2]).toMatchObject({ context: { mode: 'whole_document', section_ids: [], pinned_section_ids: [] } })
  })

  it('keeps the exact request, ID and context after uncertain delivery, including reopen/new intent', async () => {
    post.mockRejectedValueOnce(new Error('Network interrupted'))
    const ui = render(<DocumentConversation {...props} />)
    await screen.findByRole('textbox')
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'My custom request' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await screen.findByText(/Could not confirm delivery/)
    const request = post.mock.calls[0]![2]
    ui.rerender(<DocumentConversation {...props} open={false} />)
    ui.rerender(<DocumentConversation {...props} intent={{ action: 'draft_questions', whole_document: true, section_ids: [], nonce: 1 }} />)
    expect((await screen.findByRole('textbox') as HTMLTextAreaElement).value).toBe('My custom request')
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))
    await screen.findByText('A reviewable test draft.')
    expect(post.mock.calls[1]![2]).toEqual(request)
  })

  it('preserves custom drafts when changing presets and keeps rejected requests editable', async () => {
    post.mockRejectedValue(Object.assign(new Error('Context too large'), { status: 422 }))
    const ui = render(<DocumentConversation {...props} />)
    fireEvent.change(await screen.findByRole('textbox'), { target: { value: 'Keep this custom wording' } })
    fireEvent.change(screen.getByRole('combobox', { name: 'Action' }), { target: { value: 'draft_tests' } })
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Keep this custom wording')
    ui.rerender(<DocumentConversation {...props} intent={{ ...props.intent, action: 'draft_questions', nonce: 1 }} />)
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Keep this custom wording')
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await screen.findByText('Context too large')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).disabled).toBe(false)
  })

  it.each(['map', 'token'] as const)('drops old responses on %s changes', async field => {
    const old = deferred<DocumentConversationView>()
    get.mockReturnValueOnce(old.promise)
    const ui = render(<DocumentConversation {...props} />)
    ui.rerender(<DocumentConversation {...props} {...{ [field]: 'new' }} />)
    await screen.findByText(/Start a discussion/)
    await act(async () => { old.resolve(completed) })
    expect(screen.queryByText('A reviewable test draft.')).toBeNull()
    expect(get.mock.calls[0]![2]?.aborted).toBe(true)
  })

  it.each(['map', 'token'] as const)('ignores a late POST after %s changes', async field => {
    const old = deferred<DocumentConversationView>()
    post.mockReturnValueOnce(old.promise)
    const ui = render(<DocumentConversation {...props} />)
    await screen.findByRole('textbox')
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    ui.rerender(<DocumentConversation {...props} {...{ [field]: 'new' }} />)
    await screen.findByText(/Start a discussion/)
    await act(async () => { old.resolve(completed) })
    expect(screen.queryByText('A reviewable test draft.')).toBeNull()
    expect(post.mock.calls[0]![3]?.aborted).toBe(true)
    expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('restores shared history from the server after remount', async () => {
    get.mockResolvedValue(completed)
    const ui = render(<DocumentConversation {...props} />)
    await screen.findByText('A reviewable test draft.')
    ui.unmount()
    render(<DocumentConversation {...props} />)
    await screen.findByText('A reviewable test draft.')
    expect(get).toHaveBeenCalledTimes(2)
    expect(post).not.toHaveBeenCalled()
  })

  it('allows explicit whole-document discussion of an empty document', async () => {
    render(<DocumentConversation {...props} nodes={[]} intent={{ ...props.intent, whole_document: true, section_ids: [] }} />)
    await screen.findByRole('textbox')
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(post).toHaveBeenCalled())
    expect(post.mock.calls[0]![2]).toMatchObject({ context: { mode: 'whole_document', section_ids: [], pinned_section_ids: [] } })
  })

  it('keeps history viewable for readers and at the turn limit', async () => {
    get.mockResolvedValue({ ...completed, turn_limit: 1 })
    const ui = render(<DocumentConversation {...props} canAsk={false} />)
    await screen.findByText('A reviewable test draft.')
    expect(screen.queryByRole('textbox')).toBeNull()
    expect(post).not.toHaveBeenCalled()
    ui.rerender(<DocumentConversation {...props} />)
    expect(screen.getByText(/reached its turn limit/)).toBeTruthy()
    expect(screen.queryByRole('textbox')).toBeNull()
  })

  it('disables sends when a selected section disappears and bounds large scope previews', async () => {
    const ui = render(<DocumentConversation {...props} />)
    await screen.findByRole('textbox')
    ui.rerender(<DocumentConversation {...props} nodes={[props.nodes[1]!]} />)
    expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText(/Removed section/)).toBeTruthy()
    const nodes = Array.from({ length: 12 }, (_, i) => ({ ...props.nodes[0]!, id: `s${i}`, title: `Section ${i}` }))
    ui.rerender(<DocumentConversation {...props} nodes={nodes} intent={{ ...props.intent, section_ids: nodes.map(node => node.id), nonce: 1 }} />)
    expect(screen.getByText('12 sections')).toBeTruthy()
    expect(document.querySelector('details')?.open).toBe(false)
  })
})
