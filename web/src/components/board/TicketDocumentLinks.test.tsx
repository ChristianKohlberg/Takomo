import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TicketDocumentLinks } from './TicketDocumentLinks'
import * as api from '@/lib/ticket-document-links'
vi.mock('@/lib/ticket-document-links', () => ({ getTicketDocumentLinks: vi.fn(), documentSections: vi.fn(), addTicketDocumentLink: vi.fn(), changeTicketDocumentLink: vi.fn(), classifyTicketDocument: vi.fn() }))
const ref: api.DocumentReference = { id: 'link', ticket: 'TK-1', mindmap: 'map', section_id: 'section', title: 'Payments', section_version: 'snapshot-version', relation: 'source', provenance: 'direct', state: 'accepted', primary: true, created_by: 'human', created_at: '2026-09-08', missing: false, stale: false }
const suggested: api.DocumentReference = { ...ref, id: 'suggestion', section_id: 'other', title: 'Retries', provenance: 'automatic', relation: 'related', state: 'suggested', primary: false, reason: 'The ticket describes retry behavior.', quote: 'Retry failed payments.' }
const empty: api.TicketDocumentLinks = { links: [], classification: null }
const props = { token: 'token', project: 'project', ticket: 'TK-1', lang: 'en' as const, canWrite: true, onChanged: vi.fn(), onError: vi.fn() }
beforeEach(() => {
  vi.clearAllMocks(); vi.mocked(api.getTicketDocumentLinks).mockResolvedValue({ links: [ref, suggested], classification: { status: 'completed' } })
  vi.mocked(api.documentSections).mockResolvedValue([{ id: 'section', title: 'Payments', parent: null, position: 0, order: 'a' }, { id: 'other', title: 'Retries', parent: 'section', position: 0, order: 'a' }])
  vi.mocked(api.addTicketDocumentLink).mockResolvedValue({}); vi.mocked(api.changeTicketDocumentLink).mockResolvedValue({}); vi.mocked(api.classifyTicketDocument).mockResolvedValue({})
})
describe('ticket document references', () => {
  it('shows accepted sources separately and accepts a explained suggestion without replacing manual links', async () => {
    render(<TicketDocumentLinks {...props} />)
    expect((await screen.findByRole('link', { name: 'Payments' })).getAttribute('href')).toBe('/projects/project/specification?view=document&section=section')
    expect(screen.getByText('Original source · Source')).toBeTruthy()
    expect(screen.getByText('The ticket describes retry behavior.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Accept' }))
    await waitFor(() => expect(api.changeTicketDocumentLink).toHaveBeenCalledWith('token', 'TK-1', 'suggestion', { state: 'accepted' }, expect.any(AbortSignal)))
    expect(api.addTicketDocumentLink).not.toHaveBeenCalled()
    expect(api.changeTicketDocumentLink).toHaveBeenCalledTimes(1)
  })
  it('adds a chosen manual reference with optional primary and allows clearing an existing primary', async () => {
    render(<TicketDocumentLinks {...props} />)
    const picker = await screen.findByRole('combobox')
    await waitFor(() => expect(picker.querySelectorAll('option')).toHaveLength(3))
    fireEvent.change(picker, { target: { value: 'other' } })
    fireEvent.click(screen.getByRole('checkbox', { name: 'Primary heading' }))
    fireEvent.click(screen.getByRole('button', { name: 'Link section' }))
    await waitFor(() => expect(api.addTicketDocumentLink).toHaveBeenCalledWith('token', 'TK-1', 'other', true, expect.any(AbortSignal)))
    await waitFor(() => expect((screen.getByRole('button', { name: 'Clear primary' }) as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(screen.getByRole('button', { name: 'Clear primary' }))
    await waitFor(() => expect(api.changeTicketDocumentLink).toHaveBeenCalledWith('token', 'TK-1', 'link', { state: 'accepted', primary: false }, expect.any(AbortSignal)))
  })
  it('changes a suggestion through an explicit replacement and blocks stale acceptance', async () => {
    vi.mocked(api.getTicketDocumentLinks).mockResolvedValue({ links: [{ ...suggested, stale: true, created_at: 1788825600000 }], classification: { status: 'completed', ambiguity: 'Both retry and recovery sections mention this behavior.' } })
    render(<TicketDocumentLinks {...props} />)
    await screen.findByText('Both retry and recovery sections mention this behavior.')
    expect((screen.getByRole('button', { name: 'Accept' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Choose another section' }))
    expect(screen.getByText('Linking a new section will dismiss the selected suggestion.')).toBeTruthy()
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'section' } })
    fireEvent.click(screen.getByRole('button', { name: 'Link section' }))
    await waitFor(() => expect(api.changeTicketDocumentLink).toHaveBeenCalledWith('token', 'TK-1', 'suggestion', { state: 'removed' }, expect.any(AbortSignal)))
    expect(api.addTicketDocumentLink).toHaveBeenCalledWith('token', 'TK-1', 'section', false, expect.any(AbortSignal))
    expect(vi.mocked(api.addTicketDocumentLink).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(api.changeTicketDocumentLink).mock.invocationCallOrder[0]!)
  })

  it('exposes an explicit possible document gap and reuses the classification request after uncertain delivery', async () => {
    vi.mocked(api.getTicketDocumentLinks).mockResolvedValue({ links: [], classification: { status: 'no_match', no_match_reason: 'No requirement covers refunds.' } })
    vi.mocked(api.classifyTicketDocument).mockRejectedValueOnce(new Error('Connection lost'))
    render(<TicketDocumentLinks {...props} />)
    await screen.findByText(/may be a documentation gap/)
    fireEvent.click(screen.getByRole('button', { name: 'Find matches again' }))
    await screen.findByText('Connection lost')
    fireEvent.click(screen.getByRole('button', { name: 'Find matches again' }))
    await waitFor(() => expect(api.classifyTicketDocument).toHaveBeenCalledTimes(2))
    expect(vi.mocked(api.classifyTicketDocument).mock.calls[1]![2]).toBe(vi.mocked(api.classifyTicketDocument).mock.calls[0]![2])
  })
  it('readers can inspect history and removed sources without mutation controls', async () => {
    vi.mocked(api.getTicketDocumentLinks).mockResolvedValue({ links: [{ ...ref, missing: true }, { ...suggested, state: 'removed' }], classification: null })
    render(<TicketDocumentLinks {...props} canWrite={false} />)
    await screen.findByText('Section removed')
    expect(screen.queryByRole('link', { name: 'Payments' })).toBeNull()
    expect(screen.queryByRole('combobox')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Remove reference' })).toBeNull()
    expect(screen.getByText('Reference history (1)')).toBeTruthy()
  })
  it('ignores old ticket responses and aborts mutations when authentication changes', async () => {
    let resolve!: (value: api.TicketDocumentLinks) => void
    vi.mocked(api.getTicketDocumentLinks).mockReturnValueOnce(new Promise(done => { resolve = done })).mockResolvedValue(empty)
    const ui = render(<TicketDocumentLinks {...props} />)
    ui.rerender(<TicketDocumentLinks {...props} ticket="TK-2" />)
    await act(async () => resolve({ links: [ref], classification: null }))
    expect(screen.queryByRole('link', { name: 'Payments' })).toBeNull()
    expect(vi.mocked(api.getTicketDocumentLinks).mock.calls[0]![2]?.aborted).toBe(true)
    await screen.findByText('No document reference yet.')
    vi.mocked(api.classifyTicketDocument).mockReturnValue(new Promise(() => {}))
    fireEvent.click(screen.getByRole('button', { name: 'Find matches again' }))
    ui.rerender(<TicketDocumentLinks {...props} token="different" ticket="TK-2" />)
    expect(vi.mocked(api.classifyTicketDocument).mock.calls[0]![3]?.aborted).toBe(true)
  })
})
