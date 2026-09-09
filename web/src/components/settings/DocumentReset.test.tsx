import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { DocumentReset } from './DocumentReset'
import { listMindmaps } from '@/lib/mindmaps'
import { listInitiatives } from '@/lib/initiatives'
import { api } from '@/lib/api'

vi.mock('@/lib/mindmaps', () => ({ listMindmaps: vi.fn() }))
vi.mock('@/lib/initiatives', () => ({ listInitiatives: vi.fn() }))
vi.mock('@/lib/api', () => ({ api: vi.fn() }))

afterEach(cleanup)
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(listMindmaps).mockResolvedValue({ items: [{ id: 'mm-one', title: 'Original', project: 'one' }], total: 1, limit: 200 } as Awaited<ReturnType<typeof listMindmaps>>)
  vi.mocked(listInitiatives).mockResolvedValue({ items: [{ id: 'ini-one', title: 'Plan', project: 'one', status: 'open' }], next_cursor: null } as Awaited<ReturnType<typeof listInitiatives>>)
  vi.mocked(api).mockResolvedValue({})
})

async function choose(kind = 'mindmaps', id = 'mm-one') {
  await screen.findByLabelText('Document')
  fireEvent.change(screen.getByLabelText('Document'), { target: { value: `${kind}/${id}` } })
  fireEvent.click(screen.getByRole('button', { name: 'Reset document…' }))
}
async function confirmFirst() {
  fireEvent.click(screen.getByRole('button', { name: 'Continue to final confirmation' }))
  await screen.findByLabelText('Type the document ID to confirm')
}

describe('DocumentReset', () => {
  it('requires the warning confirmation and the exact id before clearing one document', async () => {
    render(<DocumentReset token="admin" project="one" lang="en" />)
    await choose()
    expect(screen.getByRole('dialog').textContent).toContain('Original')
    expect(vi.mocked(api)).not.toHaveBeenCalled()
    await confirmFirst()
    const final = screen.getByRole('button', { name: 'Clear document' })
    expect((final as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Type the document ID to confirm'), { target: { value: 'doc-other' } })
    expect((final as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Type the document ID to confirm'), { target: { value: 'mm-one' } })
    fireEvent.click(final)
    await screen.findByText('Document cleared.')
    expect(api).toHaveBeenCalledExactlyOnceWith('admin', '/mindmaps/mm-one/reset', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ confirm_id: 'mm-one' }),
    })
  })

  it('cancelling either step writes nothing and reopening starts with the first confirmation', async () => {
    render(<DocumentReset token="admin" project="one" lang="en" />)
    await choose()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Reset document…' }))
    await confirmFirst()
    fireEvent.change(screen.getByLabelText('Type the document ID to confirm'), { target: { value: 'mm-one' } })
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    fireEvent.click(screen.getByRole('button', { name: 'Reset document…' }))
    expect(screen.getByText('Clear this document? (1 of 2)')).toBeTruthy()
    expect(api).not.toHaveBeenCalled()
  })

  it('keeps failed resets visible for retry and uses the initiative endpoint', async () => {
    vi.mocked(api).mockRejectedValueOnce(new Error('Reset failed'))
    render(<DocumentReset token="admin" project="one" lang="en" />)
    await choose('initiatives', 'ini-one')
    expect(screen.getByRole('dialog').textContent).toContain('folder, metadata')
    await confirmFirst()
    fireEvent.change(screen.getByLabelText('Type the document ID to confirm'), { target: { value: 'ini-one' } })
    fireEvent.click(screen.getByRole('button', { name: 'Clear document' }))
    expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'Reset failed')
    fireEvent.click(screen.getByRole('button', { name: 'Clear document' }))
    await screen.findByText('Document cleared.')
    expect(api).toHaveBeenLastCalledWith('admin', '/initiatives/ini-one/reset', expect.objectContaining({ body: '{"confirm_id":"ini-one"}' }))
  })

  it('discards an in-progress confirmation when navigating to another project', async () => {
    const { rerender } = render(<DocumentReset key="one" token="admin" project="one" lang="en" />)
    await choose()
    await confirmFirst()
    rerender(<DocumentReset key="two" token="admin" project="two" lang="en" />)
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(api).not.toHaveBeenCalled()
    await waitFor(() => expect(listMindmaps).toHaveBeenLastCalledWith('admin', { project: 'two', q: '', limit: 200 }))
  })

  it('disables reset after a failed list request and can retry', async () => {
    vi.mocked(listMindmaps).mockRejectedValueOnce(new Error('List failed'))
    render(<DocumentReset token="admin" project="one" lang="en" />)
    expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'List failed')
    expect((screen.getByRole('button', { name: 'Reset document…' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))
    await screen.findByLabelText('Document')
    expect(api).not.toHaveBeenCalled()
  })
})
