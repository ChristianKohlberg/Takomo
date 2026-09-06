import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, useLocation, useNavigate } from 'react-router'
import { Queue } from './Queue'
import { api } from '@/lib/api'
import type { Bug } from '@/lib/bugs'
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
vi.mock('./Detail', () => ({ Detail: ({ bug }: { bug: Bug }) => <article data-testid="detail">{bug.ticket.id}</article> }))
const bugs: Record<string, Bug> = {
  'demo-1': {ticket: {id:'demo-1', project:'demo', title:'Receipt crashes', type:'bug', state:'todo'}, triage:'needs_triage', severity:'high', duplicate_of:null, latest_job:null},
  'demo-2': {ticket: {id:'demo-2', project:'demo', title:'Total doubles', type:'bug', state:'todo'}, triage:'needs_triage', severity:'unknown', duplicate_of:null, latest_job:null},
  'other-1': {ticket: {id:'other-1', project:'other', title:'Foreign bug', type:'bug', state:'todo'}, triage:'needs_triage', severity:'unknown', duplicate_of:null, latest_job:null},
}
function Probe() {
  const { search } = useLocation()
  const navigate = useNavigate()
  return <><output data-testid="search">{search}</output><button onClick={() => navigate(-1)}>Back</button><button onClick={() => navigate(1)}>Forward</button></>
}
function mount(url: string, project = 'demo') {
  render(<MemoryRouter initialEntries={[url]}><Probe /><Queue token="tk" project={project} lang="en" onAuthError={vi.fn()} canWrite canReview canConfigure={false} /></MemoryRouter>)
}
const listCalls = () => vi.mocked(api).mock.calls.map(([, path]) => path).filter(path => path.startsWith('/bugs?'))
const params = (path: string) => Object.fromEntries(new URLSearchParams(path.slice(path.indexOf('?') + 1)))
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(api).mockImplementation(async (_token, path) => {
    if (path.startsWith('/bugs?')) return {items: Object.values(bugs).filter(b => b.ticket.project === 'demo'), total: 2, limit: 50, offset: 0} as never
    const id = decodeURIComponent(path.slice('/bugs/'.length))
    if (bugs[id]) return bugs[id] as never
    throw new Error(`unexpected ${path}`)
  })
})
describe('the queue as a URL', () => {
  it('opens a shared link on its project, filters and selected bug without stored state', async () => {
    localStorage.clear()
    mount('/bugs?project=demo&view=all&severity=high&q=receipt&assignee=none&research=completed&offset=50&bug=demo-2')
    await screen.findByTestId('detail')
    expect(screen.getByTestId('detail').textContent).toBe('demo-2')
    expect(params(listCalls()[0]!)).toEqual({project:'demo', view:'all', limit:'50', offset:'50', severity:'high', search:'receipt', assignee:'none', research_status:'completed'})
    expect(screen.getByRole('button', {name:'All'})).toHaveProperty('ariaPressed', 'true')
    expect(screen.getByLabelText('Severity')).toHaveProperty('value', 'high')
    expect(screen.getByLabelText('Search bugs')).toHaveProperty('value', 'receipt')
  })
  it('writes selection and filters to the URL and restores them on Back and Forward', async () => {
    mount('/bugs?project=demo')
    fireEvent.click(await screen.findByRole('button', {name: /Receipt crashes/}))
    expect(screen.getByTestId('search').textContent).toBe('?project=demo&bug=demo-1')
    expect(screen.getByTestId('detail').textContent).toBe('demo-1')
    fireEvent.click(screen.getByRole('button', {name:'Needs triage'}))
    expect(screen.getByTestId('search').textContent).toBe('?project=demo&view=needs_triage&bug=demo-1')
    await waitFor(() => expect(params(listCalls().at(-1)!).view).toBe('needs_triage'))
    fireEvent.click(screen.getByRole('button', {name:'Back'}))
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=demo&bug=demo-1'))
    await waitFor(() => expect(params(listCalls().at(-1)!).view).toBe('open'))
    expect(screen.getByRole('button', {name:'Open bugs'})).toHaveProperty('ariaPressed', 'true')
    fireEvent.click(screen.getByRole('button', {name:'Back'}))
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=demo'))
    await waitFor(() => expect(screen.queryByTestId('detail')).toBeNull())
    expect(screen.getByText('Select a bug to review its details.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', {name:'Forward'}))
    await waitFor(() => expect(screen.getByTestId('detail').textContent).toBe('demo-1'))
  })
  it('a filter change resets paging, and typing a search does not stack history entries', async () => {
    mount('/bugs?project=demo&offset=50')
    await screen.findByRole('button', {name: /Receipt crashes/})
    fireEvent.click(screen.getByRole('button', {name:'Needs triage'}))
    expect(screen.getByTestId('search').textContent).toBe('?project=demo&view=needs_triage')
    fireEvent.change(screen.getByLabelText('Search bugs'), {target:{value:'rec'}})
    fireEvent.change(screen.getByLabelText('Search bugs'), {target:{value:'receipt'}})
    expect(screen.getByTestId('search').textContent).toBe('?project=demo&view=needs_triage&q=receipt')
    await waitFor(() => expect(params(listCalls().at(-1)!)).toMatchObject({view:'needs_triage', search:'receipt', offset:'0'}))
    fireEvent.click(screen.getByRole('button', {name:'Back'}))
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=demo&offset=50'))
    expect(screen.getByLabelText('Search bugs')).toHaveProperty('value', '')
  })
  it('drops a linked bug that belongs to another project', async () => {
    mount('/bugs?project=demo&bug=other-1')
    await screen.findByRole('button', {name: /Receipt crashes/})
    await waitFor(() => expect(screen.getByTestId('search').textContent).toBe('?project=demo'))
    expect(screen.queryByTestId('detail')).toBeNull()
  })
})
