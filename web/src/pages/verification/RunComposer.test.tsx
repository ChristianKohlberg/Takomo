import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RunComposer } from './RunComposer'
const mocks = vi.hoisted(() => ({ checkSave: 'offline', planSave: 'saved', api: vi.fn(), created: vi.fn(), onError: vi.fn() }))
vi.mock('@/hooks/useCollaboration', () => ({ useCollaboration: () => ({ saveState: mocks.checkSave }) }))
vi.mock('../specification/context', () => ({ useSpecification: () => ({ token: 'token', project: 'p', map: { id: 'm' }, saveState: mocks.planSave, lang: 'en', onError: mocks.onError }) }))
vi.mock('@/lib/api', () => ({ api: mocks.api }))
const definition = { id: 'c', definition_revision: 'def-current', specification_revision: 'spec-current', definition: { title: 'Sign in', environments: [], cases: [{ id: 'case' }] } }
describe('run creation durability boundary', () => {
  beforeEach(() => { vi.clearAllMocks(); mocks.checkSave = 'offline'; mocks.planSave = 'saved'; mocks.api.mockResolvedValue(definition) })
  it('cannot capture a draft until both replicas are acknowledged', async () => {
    const { rerender } = render(<RunComposer check="c" environments={[]} close={vi.fn()} created={mocks.created} />)
    fireEvent.change(screen.getByLabelText(/Code version/), { target: { value: 'commit123' } })
    expect((screen.getByRole('button', { name: 'Create run' }) as HTMLButtonElement).disabled).toBe(true)
    expect(mocks.api).not.toHaveBeenCalled()
    mocks.checkSave = 'saved'; mocks.planSave = 'offline'
    rerender(<RunComposer check="c" environments={[]} close={vi.fn()} created={mocks.created} />)
    expect((screen.getByRole('button', { name: 'Create run' }) as HTMLButtonElement).disabled).toBe(true)
    mocks.planSave = 'saved'
    rerender(<RunComposer check="c" environments={[]} close={vi.fn()} created={mocks.created} />)
    await waitFor(() => expect((screen.getByRole('button', { name: 'Create run' }) as HTMLButtonElement).disabled).toBe(false))
    expect(mocks.api).toHaveBeenCalledWith('token', '/checks/c/definition')
    mocks.api.mockResolvedValueOnce({ id: 'run-1' })
    fireEvent.click(screen.getByRole('button', { name: 'Create run' }))
    await waitFor(() => expect(mocks.created).toHaveBeenCalledWith({ id: 'run-1' }))
    const options = mocks.api.mock.calls.at(-1)![2]
    expect(JSON.parse(options.body)).toMatchObject({ code_ref: 'commit123', definitions: [{ check: 'c', definition_revision: 'def-current', specification_revision: 'spec-current' }] })
  })
  it('reuses the creation key after an uncertain network response', async () => {
    mocks.checkSave = 'saved'
    render(<RunComposer check="c" environments={[]} close={vi.fn()} created={mocks.created} />)
    fireEvent.change(screen.getByLabelText(/Code version/), { target: { value: 'commit123' } })
    const button = screen.getByRole('button', { name: 'Create run' })
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false))
    mocks.api.mockRejectedValueOnce(new Error('Connection lost'))
    fireEvent.click(button)
    await waitFor(() => expect(mocks.onError).toHaveBeenCalled())
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(button)
    await waitFor(() => expect(mocks.created).toHaveBeenCalled())
    const requests = mocks.api.mock.calls.filter(call => call[1].endsWith('/test-runs'))
    expect(JSON.parse(requests[0]![2].body).idempotency_key).toBe(JSON.parse(requests[1]![2].body).idempotency_key)
  })
})
