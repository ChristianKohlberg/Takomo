import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Inspector } from './Inspector'
import { getAgentJob, listAgentJobs, type AgentJob, type AgentJobDetail, type AgentJobList } from '@/lib/agent-queues'

vi.mock('@/lib/agent-queues', async importOriginal => ({ ...await importOriginal<typeof import('@/lib/agent-queues')>(), listAgentJobs: vi.fn(), getAgentJob: vi.fn() }))
const job: AgentJob = {
  id: 'aj-one', conversation_id: 'ac-one', project: 'demo', mindmap: 'mm-one', node: 'node-one',
  section_title: 'Payment reminders', status: 'running', requested_by: 'human:one', source_revision: 'abc',
  created_at: 1788624000000, finished_at: null, lease_expires_at: 1788624060000, deadline: 1788624900000,
  service_id: 'worker-one', conversation_service_id: 'worker-one', attempt_id: 'attempt-one', thread_id: 'thread-one', turn_id: 'turn-one', error: null,
}
const list = (items: AgentJob[] = [job]): AgentJobList => ({ items, total: items.length, limit: 100, counts: { queued: 0, running: 1, completed: 0, failed: 0, cancelled: 0 } })
const detail = (j = job): AgentJobDetail => ({ job: { ...j, prompt: 'Grill the timing rule', snapshot: '# Payment reminders\nWithin 30 minutes.', response: '**Clarify** the timezone.' }, messages: [] })
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}
beforeEach(() => {
  vi.mocked(listAgentJobs).mockReset().mockResolvedValue(list())
  vi.mocked(getAgentJob).mockReset().mockResolvedValue(detail())
})
afterEach(() => { cleanup(); vi.useRealTimers() })

describe('Agent queue inspector', () => {
  it('filters cancelled bug research and opens its ticket instead of a document', async () => {
    const cancelled: AgentJob = {...job, kind: 'bug_research', ticket_id: 'demo-bug', mindmap: null, status: 'cancelled'}
    vi.mocked(listAgentJobs).mockResolvedValue({...list([cancelled]), counts: {queued: 0, running: 0, completed: 0, failed: 0, cancelled: 1}})
    vi.mocked(getAgentJob).mockResolvedValue(detail(cancelled))
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    fireEvent.change(screen.getByRole('combobox', {name: 'Status'}), {target: {value: 'cancelled'}})
    fireEvent.click(await screen.findByRole('button', {name: /demo-bug/}))
    const pane = await screen.findByRole('region', {name: 'Request details'})
    const link = await within(pane).findByRole('link', {name: 'Open bug ticket'})
    expect(link.getAttribute('href')).toBe('/board#t=demo-bug')
    expect(listAgentJobs).toHaveBeenLastCalledWith('reader', 'demo', 'cancelled', expect.any(AbortSignal))
    expect(within(pane).queryByRole('link', {name: 'Open section'})).toBeNull()
  })
  it('shows saved request context and execution identifiers without mutation controls', async () => {
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: /Payment reminders/ }))
    const pane = await screen.findByRole('region', { name: 'Request details' })
    await within(pane).findByText('thread-one')
    expect(within(pane).getAllByText('worker-one', { selector: 'dd' })).toHaveLength(2)
    expect(within(pane).getAllByText(/2026/, { selector: 'dd' }).length).toBeGreaterThan(0)
    expect(within(pane).getByRole('link', { name: 'Open section' }).getAttribute('href')).toBe('/projects/demo/specification?view=document&section=node-one')
    expect(within(pane).getByText('Grill the timing rule')).toBeTruthy()
    expect(within(pane).getByText(/Within 30 minutes/)).toBeTruthy()
    expect(within(pane).getByText('Clarify')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /retry|cancel/i })).toBeNull()
  })
  it('ignores old filter responses and clears selected details when the filter changes', async () => {
    const pending = deferred<AgentJobList>()
    vi.mocked(listAgentJobs).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(list([{ ...job, id: 'aj-failed', section_title: 'Failed section', status: 'failed' }]))
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    fireEvent.change(screen.getByRole('combobox', { name: 'Status' }), { target: { value: 'failed' } })
    await screen.findByRole('button', { name: /Failed section/ })
    await act(async () => pending.resolve(list()))
    expect(screen.queryByRole('button', { name: /Payment reminders/ })).toBeNull()
    expect(listAgentJobs).toHaveBeenLastCalledWith('reader', 'demo', 'failed', expect.any(AbortSignal))
  })
  it('does not replace the selected detail with a late response from a previous selection', async () => {
    const pending = deferred<AgentJobDetail>()
    const other = { ...job, id: 'aj-two', section_title: 'Second section', thread_id: 'thread-two' }
    vi.mocked(listAgentJobs).mockResolvedValue(list([job, other]))
    vi.mocked(getAgentJob).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(detail(other))
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: /Payment reminders/ }))
    fireEvent.click(screen.getByRole('button', { name: /Second section/ }))
    await screen.findByText('thread-two')
    await act(async () => pending.resolve(detail()))
    expect(screen.queryByText('thread-one')).toBeNull()
  })
  it('polls running requests, refreshes the selected details, and can pause polling', async () => {
    vi.useFakeTimers()
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    await act(async () => {})
    fireEvent.click(screen.getByRole('button', { name: /Payment reminders/ }))
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(listAgentJobs).toHaveBeenCalledTimes(2)
    expect(getAgentJob).toHaveBeenCalledTimes(2)
    fireEvent.click(screen.getByRole('checkbox', { name: 'Refresh every 3 seconds' }))
    await act(async () => {})
    const calls = vi.mocked(listAgentJobs).mock.calls.length
    await act(async () => { await vi.advanceTimersByTimeAsync(6000) })
    expect(listAgentJobs).toHaveBeenCalledTimes(calls)
  })
  it('keeps stale data visible with an error and distinguishes permission failures from expired credentials', async () => {
    const onAuthError = vi.fn()
    vi.mocked(listAgentJobs).mockResolvedValueOnce(list()).mockRejectedValueOnce(Object.assign(new Error('Project access denied'), { status: 403 })).mockRejectedValueOnce(Object.assign(new Error('auth'), { status: 401 }))
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={onAuthError} />)
    await screen.findByRole('button', { name: /Payment reminders/ })
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    expect((await screen.findByRole('alert')).textContent).toContain('displayed data may be out of date')
    expect(screen.getByRole('button', { name: /Payment reminders/ })).toBeTruthy()
    expect(onAuthError).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(onAuthError).toHaveBeenCalledOnce())
  })
  it('reports truncation and explains a queued request bound to a worker', async () => {
    const queued = { ...job, status: 'queued' as const, service_id: null }
    vi.mocked(listAgentJobs).mockResolvedValue({ ...list([queued]), total: 101 })
    vi.mocked(getAgentJob).mockResolvedValue(detail(queued))
    render(<Inspector token="reader" project="demo" lang="en" onAuthError={vi.fn()} />)
    expect(await screen.findByText(/Only the latest 100/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /Payment reminders/ }))
    expect(await screen.findByText('Waiting for the conversation worker to claim this request.')).toBeTruthy()
  })
})
