import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { Report } from './Report'
import { api } from '@/lib/api'
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
beforeEach(() => vi.resetAllMocks())
describe('bug reporting', () => {
  it('requires a title and an observed problem, allowing either description or actual behavior', () => {
    render(<Report token="tk" project="demo" lang="en" onCreated={vi.fn()} onCancel={vi.fn()} />)
    const create = screen.getByRole('button', {name: 'Create bug ticket'})
    fireEvent.change(screen.getByLabelText('Title'), {target: {value: 'Receipt crashes'}})
    expect(create).toHaveProperty('disabled', true)
    fireEvent.change(screen.getByLabelText('Description'), {target: {value: '   '}})
    expect(create).toHaveProperty('disabled', true)
    fireEvent.change(screen.getByLabelText('Description'), {target: {value: 'Opening a receipt closes the page'}})
    expect(create).toHaveProperty('disabled', false)
    fireEvent.change(screen.getByLabelText('Description'), {target: {value: ''}})
    fireEvent.change(screen.getByLabelText('Actual behavior'), {target: {value: 'The page closes'}})
    expect(create).toHaveProperty('disabled', false)
    expect(screen.getByLabelText('Description')).toHaveProperty('required', false)
    expect(api).not.toHaveBeenCalled()
  })
  it('creates one ordinary bug ticket and never queues research', async () => {
    vi.mocked(api).mockResolvedValue({id: 'demo-1'})
    const done = vi.fn()
    render(<Report token="tk" project="demo" lang="en" onCreated={done} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText('Title'), {target: {value: 'Receipt crashes'}})
    fireEvent.change(screen.getByLabelText('Actual behavior'), {target: {value: 'The page closes unexpectedly'}})
    fireEvent.change(screen.getByLabelText('Steps to reproduce'), {target: {value: 'Open a receipt'}})
    fireEvent.click(screen.getByRole('button', {name: 'Create bug ticket'}))
    await waitFor(() => expect(done).toHaveBeenCalled())
    expect(api).toHaveBeenCalledTimes(1)
    const [, path, options] = vi.mocked(api).mock.calls[0]!
    expect(path).toBe('/tickets')
    expect(JSON.parse(options!.body as string)).toMatchObject({project: 'demo', type: 'bug', title: 'Receipt crashes', priority: 'normal'})
    expect(options!.headers!['Idempotency-Key']).toBeTruthy()
  })
  it('retries an ambiguous response with the same exact ticket request', async () => {
    vi.mocked(api).mockRejectedValueOnce(new Error('Connection lost')).mockResolvedValueOnce({id: 'demo-1'})
    render(<Report token="tk" project="demo" lang="en" onCreated={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText('Title'), {target: {value: 'Receipt crashes'}})
    fireEvent.change(screen.getByLabelText('Actual behavior'), {target: {value: 'The page closes unexpectedly'}})
    fireEvent.click(screen.getByRole('button', {name: 'Create bug ticket'}))
    expect((await screen.findByRole('alert')).textContent).toContain('Connection lost')
    fireEvent.click(screen.getByRole('button', {name: 'Create bug ticket'}))
    await waitFor(() => expect(api).toHaveBeenCalledTimes(2))
    expect(vi.mocked(api).mock.calls[1]).toEqual(vi.mocked(api).mock.calls[0])
  })
})
