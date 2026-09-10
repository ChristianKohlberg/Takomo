import { beforeEach, describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { App } from './App'
import { getTicket, getWorkflow, listTickets, type Ticket } from '@/lib/board'
import { listQuestions } from '@/lib/questions'
import { fetchRoadmap, type Roadmap } from '@/lib/roadmap'
import type { NavRailProps } from '@/components/NavRail'
import type { ReactNode } from 'react'

const live = vi.hoisted(() => ({ listeners: new Set<(event?: { type: 'refresh'; topics?: ('tickets' | 'projects' | 'inbox')[] }) => Promise<unknown>>() }))
vi.mock('@/hooks/useProjectUpdates', async importOriginal => {
  const actual = await importOriginal<typeof import('@/hooks/useProjectUpdates')>()
  const React = await import('react')
  return { ...actual, useProjectUpdates: (_token: string, _project: string, callback: (event?: { type: 'refresh'; topics?: ('tickets' | 'projects' | 'inbox')[] }) => Promise<unknown>) => {
    const latest = React.useRef(callback); latest.current = callback
    React.useEffect(() => { const listener = (event?: { type: 'refresh'; topics?: ('tickets' | 'projects' | 'inbox')[] }) => latest.current(event); live.listeners.add(listener); return () => { live.listeners.delete(listener) } }, [])
    return true
  } }
})
async function poll() {
  await act(async () => { await Promise.all([...live.listeners].map(listener => listener({ type: 'refresh' }))) })
}

const toast = vi.hoisted(() => vi.fn())
vi.mock('@/components/Toaster', () => ({ useToast: () => ({ toast }) }))
vi.mock('@/lib/initiatives', () => ({
  listProjects: vi.fn(async () => [{ id: 'first', name: 'First' }, { id: 'second', name: 'Second' }]),
  whoami: vi.fn(async () => ({ actor: 'captain', scopes: ['write'] })),
  listInitiatives: vi.fn(async () => ({ items: [] })),
  countWaiting: vi.fn(() => 0),
}))
vi.mock('@/lib/board', () => ({
  getWorkflow: vi.fn(async () => ({ initial: 'brief', states: [{ id: 'brief' }, { id: 'shipped', terminal: true }] })),
  listTickets: vi.fn(async () => []), getTicket: vi.fn(), getEvents: vi.fn(), hasEvents: vi.fn(),
}))
vi.mock('@/lib/users', () => ({ listUsers: vi.fn(async () => ({ items: [] })) }))
vi.mock('@/lib/questions', () => ({ listQuestions: vi.fn(async () => []), askQuestion: vi.fn(), answerQuestion: vi.fn() }))
vi.mock('@/lib/roadmap', async (importOriginal) => ({ ...await importOriginal<typeof import('@/lib/roadmap')>(), fetchRoadmap: vi.fn() }))
vi.mock('@/components/AppShell', () => ({ AppShell: ({ rail, children }: { rail: NavRailProps; children: ReactNode }) => <><span data-testid="current">{rail.current}</span><span data-testid="inbox-count">{rail.badges?.inbox}</span><button onClick={() => rail.onProject?.('second')}>Switch project</button>{children}</> }))
vi.mock('@/components/board/AskDrawer', () => ({ AskDrawer: () => null }))
vi.mock('@/components/board/InboxDrawer', () => ({ InboxDrawer: () => null }))
vi.mock('@/components/board/DetailPanel', () => ({ DetailPanel: ({ ticket, onClose }: { ticket: Ticket | null; onClose: () => void }) => ticket ? <div role="dialog" aria-label="Epic detail">{ticket.title}<button onClick={onClose}>Close detail</button></div> : null }))

function roadmap(title: string): Roadmap {
  return { epics: [{ id: title.toLowerCase().replaceAll(' ', '-'), title, state: 'brief', state_category: 'backlog', priority: 'normal', flags: [], total: 2, done: 0, percent: 0, ready: 0, backlog: 2, awaiting_answer: 0 }], lanes: [] } as unknown as Roadmap
}
function mount() { return render(<MemoryRouter><App surface="epics" /></MemoryRouter>) }
beforeEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
  window.history.replaceState(null, '', '/epics')
  localStorage.clear()
  localStorage.setItem('takomo.token', 'tk-test')
  localStorage.setItem('takomo.project', 'first')
})

describe('Epics page', () => {
  it('shows the dedicated surface without the board filter bank', async () => {
    vi.mocked(fetchRoadmap).mockResolvedValue(roadmap('First epic'))
    mount()
    await screen.findByText('First epic')
    expect(screen.getByTestId('current').textContent).toBe('epics')
    expect(screen.queryByRole('group', { name: 'View' })).toBeNull()
    expect(document.getElementById('tickfilter')).toBeNull()
    expect(screen.getByRole('button', { name: 'New epic' })).toBeTruthy()
  })

  it('distinguishes failed loading from empty and retries', async () => {
    vi.mocked(fetchRoadmap).mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(roadmap('Recovered epic'))
    mount()
    expect((await screen.findByRole('alert')).textContent).toContain('Could not load epics.')
    expect(screen.queryByText('No epics yet')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    await screen.findByText('Recovered epic')
    expect(fetchRoadmap).toHaveBeenCalledTimes(2)
  })

  it('ignores an old project response after the selected project changes', async () => {
    let finishFirst!: (value: Roadmap) => void
    vi.mocked(fetchRoadmap).mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve })).mockResolvedValueOnce(roadmap('Second epic'))
    mount()
    await waitFor(() => expect(fetchRoadmap).toHaveBeenCalledWith('tk-test', 'first'))
    fireEvent.click(screen.getByRole('button', { name: 'Switch project' }))
    await screen.findByText('Second epic')
    finishFirst(roadmap('Old epic'))
    await waitFor(() => expect(screen.queryByText('Old epic')).toBeNull())
    expect(screen.getByText('Second epic')).toBeTruthy()
  })

  it('keeps the most recently opened epic when detail reads resolve out of order', async () => {
    const first = roadmap('First epic')
    vi.mocked(fetchRoadmap).mockResolvedValue({ ...first, epics: [...first.epics, ...roadmap('Second epic').epics] })
    let finishFirst!: (value: Ticket) => void
    vi.mocked(getTicket).mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve })).mockResolvedValue({ id: 'second-epic', project: 'first', title: 'Second detail', state: 'brief' })
    mount()
    fireEvent.click(await screen.findByText('First epic'))
    fireEvent.click(screen.getByText('Second epic'))
    expect((await screen.findByRole('dialog', { name: 'Epic detail' })).textContent).toContain('Second detail')
    await act(async () => finishFirst({ id: 'first-epic', project: 'first', title: 'Old detail', state: 'brief' }))
    expect(screen.getByRole('dialog', { name: 'Epic detail' }).textContent).toContain('Second detail')
    expect(screen.queryByText('Old detail')).toBeNull()
  })

  it('does not reopen a closed detail panel when an earlier read finishes late', async () => {
    const first = roadmap('First epic')
    vi.mocked(fetchRoadmap).mockResolvedValue({ ...first, epics: [...first.epics, ...roadmap('Second epic').epics] })
    let finishRead!: (value: Ticket) => void
    vi.mocked(getTicket).mockImplementationOnce(() => new Promise((resolve) => { finishRead = resolve })).mockResolvedValue({ id: 'second-epic', project: 'first', title: 'Second detail', state: 'brief' })
    mount()
    fireEvent.click(await screen.findByText('First epic'))
    fireEvent.click(screen.getByText('Second epic'))
    await screen.findByRole('dialog', { name: 'Epic detail' })
    fireEvent.click(screen.getByRole('button', { name: 'Close detail' }))
    await act(async () => finishRead({ id: 'first-epic', project: 'first', title: 'Late detail', state: 'brief' }))
    expect(screen.queryByRole('dialog', { name: 'Epic detail' })).toBeNull()
  })

  it('preserves search through a failed background refresh and retry', async () => {
    vi.mocked(fetchRoadmap).mockResolvedValueOnce(roadmap('First epic')).mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(roadmap('First epic'))
    mount()
    await screen.findByText('First epic')
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search epics' }), { target: { value: 'First' } })
    await act(async () => poll())
    expect((await screen.findByRole('alert')).textContent).toContain('Showing the last loaded version')
    expect(screen.getByRole('searchbox', { name: 'Search epics' })).toHaveProperty('value', 'First')
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull())
    expect(screen.getByRole('searchbox', { name: 'Search epics' })).toHaveProperty('value', 'First')
  })


  it.each(['workflow', 'tickets'] as const)('preserves filters after a background %s read fails', async (resource) => {
    vi.mocked(fetchRoadmap).mockResolvedValue(roadmap('First epic'))
    mount()
    await screen.findByText('First epic')
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search epics' }), { target: { value: 'First' } })
    if (resource === 'workflow') vi.mocked(getWorkflow).mockRejectedValueOnce(new Error('offline'))
    else vi.mocked(listTickets).mockRejectedValueOnce(new Error('offline'))
    await act(async () => poll())
    expect((await screen.findByRole('alert')).textContent).toContain('Showing the last loaded version')
    expect(screen.getByRole('searchbox', { name: 'Search epics' })).toHaveProperty('value', 'First')
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull())
    expect(screen.getByRole('searchbox', { name: 'Search epics' })).toHaveProperty('value', 'First')
  })

})

it('opens a shared ticket in its URL project and removes its link on close', async () => {
  window.history.replaceState(null, '', '/epics?project=second#t=second-ticket')
  vi.mocked(fetchRoadmap).mockResolvedValue(roadmap('Second epic'))
  vi.mocked(getTicket).mockResolvedValue({ id: 'second-ticket', title: 'Shared ticket', project: 'second', state: 'brief' })
  mount()
  expect(await screen.findByRole('dialog', { name: 'Epic detail' })).toHaveProperty('textContent', expect.stringContaining('Shared ticket'))
  expect(getWorkflow).toHaveBeenCalledWith('tk-test', 'second')
  fireEvent.click(screen.getByRole('button', { name: 'Close detail' }))
  expect(window.location.hash).toBe('')
  expect(window.location.search).toBe('?project=second')
})

it('refreshes board questions independently from ticket and workflow updates', async () => {
  vi.mocked(fetchRoadmap).mockResolvedValue(roadmap('First epic'))
  mount()
  await screen.findByText('First epic')
  await waitFor(() => expect(listQuestions).toHaveBeenCalledTimes(1))
  const workflowReads = vi.mocked(getWorkflow).mock.calls.length
  const ticketReads = vi.mocked(listTickets).mock.calls.length
  await act(async () => { await Promise.all([...live.listeners].map(listener => listener({ type: 'refresh', topics: ['tickets'] }))) })
  await waitFor(() => expect(listTickets).toHaveBeenCalledTimes(ticketReads + 1), { timeout: 2000 })
  expect(listQuestions).toHaveBeenCalledTimes(1)
  expect(getWorkflow).toHaveBeenCalledTimes(workflowReads)
  vi.mocked(listQuestions).mockResolvedValueOnce([{ id: 'question', ticket: 'first-epic', awaiting: 'human', mode: 'blocking' }] as never)
  await act(async () => { await Promise.all([...live.listeners].map(listener => listener({ type: 'refresh', topics: ['inbox'] }))) })
  await waitFor(() => expect(screen.getByTestId('inbox-count').textContent).toBe('1'), { timeout: 2000 })
  expect(listQuestions).toHaveBeenCalledTimes(2)
  expect(listTickets).toHaveBeenCalledTimes(ticketReads + 1)
  expect(getWorkflow).toHaveBeenCalledTimes(workflowReads)
})
