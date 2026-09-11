import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { DocumentClassificationPolicy } from './DocumentClassificationPolicy'
import { getClassificationPolicy, saveClassificationPolicy, classifyProjectDocuments } from '@/lib/ticket-document-links'
vi.mock('@/lib/ticket-document-links', () => ({ getClassificationPolicy: vi.fn(), saveClassificationPolicy: vi.fn(), classifyProjectDocuments: vi.fn() }))
const props = { token: 'token', project: 'project', lang: 'en' as const, readOnly: false, canClassify: true }
beforeEach(() => { vi.clearAllMocks(); vi.mocked(getClassificationPolicy).mockResolvedValue({ mode: 'suggest' }); vi.mocked(saveClassificationPolicy).mockResolvedValue({}); vi.mocked(classifyProjectDocuments).mockResolvedValue({ scheduled: 3 }) })
describe('project document classification settings', () => {
  it('defaults to review, saves explicit policy and only schedules backfill after a click', async () => {
    render(<DocumentClassificationPolicy {...props} />)
    await waitFor(() => expect((screen.getByRole('combobox', { name: 'Document classification' }) as HTMLSelectElement).disabled).toBe(false))
    expect(classifyProjectDocuments).not.toHaveBeenCalled()
    fireEvent.change(screen.getByRole('combobox', { name: 'Document classification' }), { target: { value: 'auto_apply_clear' } })
    expect(screen.getByRole('option', { name: /unique title matches/ })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Save classification policy' }))
    await screen.findByText('Classification policy saved.')
    expect(saveClassificationPolicy).toHaveBeenCalledWith('token', 'project', 'auto_apply_clear', expect.any(AbortSignal), 'automatic')
    fireEvent.click(screen.getByRole('button', { name: 'Find matches for tickets without references' }))
    await screen.findByText('3 tickets queued for matching.')
  })
  it('saves scheduling independently and disables requests until saved and while off', async () => {
    render(<DocumentClassificationPolicy {...props} />)
    const select = screen.getByRole('combobox', { name: 'Classification scheduling' }) as HTMLSelectElement
    await waitFor(() => expect(select.disabled).toBe(false))
    const run = screen.getByRole('button', { name: 'Find matches for tickets without references' }) as HTMLButtonElement
    vi.mocked(saveClassificationPolicy).mockResolvedValueOnce({ cancelled: 398 })
    fireEvent.change(select, { target: { value: 'off' } })
    expect(run.disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Save classification policy' }))
    await screen.findByText('Classification policy saved.')
    expect(saveClassificationPolicy).toHaveBeenCalledWith('token', 'project', 'suggest', expect.any(AbortSignal), 'off')
    expect(await screen.findByText('398 queued jobs cancelled.')).toBeTruthy()
    expect(run.disabled).toBe(true)
    fireEvent.change(select, { target: { value: 'manual' } })
    expect(run.disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Save classification policy' }))
    await waitFor(() => expect(run.disabled).toBe(false))
    expect(classifyProjectDocuments).not.toHaveBeenCalled()
  })
  it('readers see policy without mutation controls', async () => {
    render(<DocumentClassificationPolicy {...props} readOnly canClassify={false} />)
    await waitFor(() => expect(getClassificationPolicy).toHaveBeenCalled())
    expect((screen.getByRole('combobox', { name: 'Document classification' }) as HTMLSelectElement).disabled).toBe(true)
    expect(screen.queryByRole('button')).toBeNull()
  })
  it('does not leak old project settings after a scope change', async () => {
    let resolve!: (value: { mode: 'auto_apply_clear' }) => void
    vi.mocked(getClassificationPolicy).mockReturnValueOnce(new Promise(done => { resolve = done }))
    const ui = render(<DocumentClassificationPolicy {...props} />)
    ui.rerender(<DocumentClassificationPolicy {...props} project="other" />)
    await act(async () => resolve({ mode: 'auto_apply_clear' }))
    expect((screen.getByRole('combobox', { name: 'Document classification' }) as HTMLSelectElement).value).toBe('suggest')
    expect(vi.mocked(getClassificationPolicy).mock.calls[0]![2]?.aborted).toBe(true)
  })
})
