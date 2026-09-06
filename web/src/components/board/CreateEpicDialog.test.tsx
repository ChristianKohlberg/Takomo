import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { CreateEpicDialog } from './CreateEpicDialog'
import { api } from '@/lib/api'

vi.mock('@/lib/api', () => ({ api: vi.fn() }))
const ticket = { id: 'demo-9', project: 'demo', title: 'Launch', type: 'epic', state: 'brief' }

beforeEach(() => vi.resetAllMocks())
function mount() {
  const onCreated = vi.fn()
  render(<CreateEpicDialog open onOpenChange={vi.fn()} token="tk-test" project="demo" lang="en" onCreated={onCreated} />)
  return onCreated
}

describe('CreateEpicDialog', () => {
  it('creates a real epic in the selected project using the server workflow initial state', async () => {
    vi.mocked(api).mockResolvedValue(ticket)
    const onCreated = mount()
    expect(screen.getByRole('button', { name: 'Create epic' })).toHaveProperty('disabled', true)
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: '  Launch  ' } })
    fireEvent.change(screen.getByLabelText('Description (optional)'), { target: { value: 'A useful outcome' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create epic' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(ticket))
    const [token, path, options] = vi.mocked(api).mock.calls[0]!
    expect(token).toBe('tk-test')
    expect(path).toBe('/tickets')
    expect(options?.method).toBe('POST')
    expect(JSON.parse(options?.body as string)).toEqual({ project: 'demo', type: 'epic', title: 'Launch', body: 'A useful outcome' })
  })

  it('keeps the draft and shows server errors for retry', async () => {
    vi.mocked(api).mockRejectedValueOnce(new Error('This project is archived.')).mockResolvedValueOnce(ticket)
    const onCreated = mount()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Launch' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create epic' }))
    expect((await screen.findByRole('alert')).textContent).toContain('This project is archived.')
    expect(onCreated).not.toHaveBeenCalled()
    expect(screen.getByLabelText('Title')).toHaveProperty('value', 'Launch')
    fireEvent.click(screen.getByRole('button', { name: 'Create epic' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(ticket))
  })
})
