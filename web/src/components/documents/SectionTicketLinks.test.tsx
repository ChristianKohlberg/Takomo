import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SectionTicketLinks } from './SectionTicketLinks'
import { getProjectDocumentLinks, type ReferencePage } from '@/lib/ticket-document-links'
vi.mock('@/lib/ticket-document-links', () => ({ getProjectDocumentLinks: vi.fn() }))
const props = { token: 'token', project: 'project', section: 'section', lang: 'en' as const }
const page: ReferencePage = { items: [{ id: 'link', ticket: 'TK-1', ticket_title: 'Handle failures', ticket_state: 'todo', mindmap: 'map', section_id: 'section', title: 'Errors', section_version: 'v1', primary: false, provenance: 'manual', relation: 'related', state: 'accepted', created_by: 'human', created_at: 'today', missing: false, stale: false }], total: 3, limit: 1 }
beforeEach(() => { vi.clearAllMocks(); vi.mocked(getProjectDocumentLinks).mockResolvedValue(page) })
describe('section reverse ticket links', () => {
  it('fetches only when expanded and states partial coverage explicitly', async () => {
    render(<SectionTicketLinks {...props} />)
    expect(getProjectDocumentLinks).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Associated tickets' }))
    expect((await screen.findByRole('link', { name: 'Handle failures' })).getAttribute('href')).toBe('/board?project=project#t=TK-1')
    expect(getProjectDocumentLinks).toHaveBeenCalledWith('token', 'project', expect.any(AbortSignal), 'section', 0)
    expect(screen.getByText('Showing 1 of 3 references.')).toBeTruthy()
    vi.mocked(getProjectDocumentLinks).mockResolvedValue({ ...page, items: [{ ...page.items[0]!, id: 'next-link', ticket: 'TK-2', ticket_title: 'More failures' }], offset: 1 })
    fireEvent.click(screen.getByRole('button', { name: 'Load more' }))
    await screen.findByRole('link', { name: 'More failures' })
    expect(screen.getByRole('link', { name: 'Handle failures' })).toBeTruthy()
    expect(getProjectDocumentLinks).toHaveBeenLastCalledWith('token', 'project', expect.any(AbortSignal), 'section', 1)
  })
  it('discards a response for a previously expanded section', async () => {
    let resolve!: (value: ReferencePage) => void
    vi.mocked(getProjectDocumentLinks).mockReturnValueOnce(new Promise(done => { resolve = done })).mockResolvedValue({ items: [], total: 0, limit: 500 })
    const ui = render(<SectionTicketLinks {...props} />)
    fireEvent.click(screen.getByRole('button', { name: 'Associated tickets' }))
    ui.rerender(<SectionTicketLinks {...props} section="other" />)
    await act(async () => resolve(page))
    expect(screen.queryByRole('link')).toBeNull()
    expect(vi.mocked(getProjectDocumentLinks).mock.calls[0]![2]?.aborted).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Associated tickets' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Associated tickets (0)' })).toBeTruthy())
  })
})
