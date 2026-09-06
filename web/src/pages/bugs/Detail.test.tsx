import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { Detail } from './Detail'
import { api } from '@/lib/api'
import type { Bug } from '@/lib/bugs'
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
const bug: Bug = {ticket: {id:'demo-1', project:'demo', title:'Receipt crashes', type:'bug', state:'todo'}, triage:'needs_triage', severity:'unknown', duplicate_of:null, latest_job:null}
let jobs: {id: string; status: string; created_at: number; response?: string; prompt?: string; snapshot?: string}[]
beforeEach(() => {
  vi.resetAllMocks(); jobs = []
  vi.mocked(api).mockImplementation(async (_token, path, opts) => {
    if (opts?.method) return {} as never
    if (path.endsWith('/bug-research-config')) return {repository:'takomo', revision:'main', enabled:true} as never
    return {jobs} as never
  })
})
function mount() { render(<Detail bug={bug} token="tk" lang="en" canWrite canReview canConfigure={false} refresh={vi.fn()} onAuthError={vi.fn()} />) }
describe('explicit bug research', () => {
  it('does not start on display; shows repository before the explicit request', async () => {
    mount()
    await screen.findByText('Repository: takomo · Revision: main')
    expect(vi.mocked(api).mock.calls.filter(([, , opts]) => opts?.method)).toHaveLength(0)
    fireEvent.click(screen.getByRole('button', {name: 'Research with Codex'}))
    await waitFor(() => expect(vi.mocked(api).mock.calls.find(([, path, opts]) => path === '/bugs/demo-1/research' && opts?.method === 'POST')).toBeTruthy())
    const request = vi.mocked(api).mock.calls.find(([, , opts]) => opts?.method === 'POST')!
    expect(JSON.parse(request[2]!.body as string).request_id).toBeTruthy()
  })
  it('offers steering and cancellation for active research instead of a second start', async () => {
    jobs = [{id:'aj-1', status:'running', created_at:1788690000000}]
    mount()
    await screen.findByRole('button', {name: 'Send guidance'})
    expect(screen.queryByRole('button', {name:'Research with Codex'})).toBeNull()
    fireEvent.change(screen.getByLabelText('Guidance for Codex'), {target:{value:'Check receipt import'}})
    fireEvent.click(screen.getByRole('button', {name:'Send guidance'}))
    await waitFor(() => expect(vi.mocked(api).mock.calls.some(([,path]) => path === '/agent-jobs/aj-1/steer')).toBe(true))
    await waitFor(() => expect(screen.getByRole('button', {name:'Cancel'})).toHaveProperty('disabled',false))
    fireEvent.click(screen.getByRole('button', {name:'Cancel'}))
    await waitFor(() => expect(vi.mocked(api).mock.calls.some(([,path]) => path === '/agent-jobs/aj-1/cancel')).toBe(true))
  })
  it('keeps completed research separate from human triage', async () => {
    jobs = [{id:'aj-1', status:'completed', created_at:1788690000000,response:'## Hypotheses\nPossible null receipt.',prompt:'Check receipt import',snapshot:'Receipt crashes at original revision'}]
    mount()
    await screen.findByRole('button', {name:'Research again'})
    expect(screen.getByText('Research input')).toBeTruthy()
    expect(screen.getByText('Check receipt import')).toBeTruthy()
    expect(screen.getByText('Receipt crashes at original revision')).toBeTruthy()
    expect(screen.queryByTestId('snapshot')).toBeNull()
    expect(screen.getByLabelText('Triage')).toHaveProperty('value','needs_triage')
    expect(vi.mocked(api).mock.calls.some(([, ,opts]) => opts?.method === 'PATCH')).toBe(false)
    fireEvent.change(screen.getByLabelText('Triage'), {target:{value:'confirmed'}})
    fireEvent.click(screen.getByRole('button', {name:'Save'}))
    await waitFor(() => expect(vi.mocked(api).mock.calls.some(([,path,opts]) => path === '/bugs/demo-1' && opts?.method === 'PATCH' && JSON.parse(opts.body as string).triage === 'confirmed')).toBe(true))
  })
})

describe('review note', () => {
  it('an emptied note is sent as an empty string so the server clears it', async () => {
    render(<Detail bug={{...bug, note: 'Reviewed evidence'}} token="tk" lang="en" canWrite canReview canConfigure={false} refresh={vi.fn()} onAuthError={vi.fn()} />)
    await screen.findByText('Repository: takomo · Revision: main')
    expect(screen.getByLabelText('Review note')).toHaveProperty('value', 'Reviewed evidence')
    fireEvent.change(screen.getByLabelText('Review note'), {target:{value:''}})
    fireEvent.click(screen.getByRole('button', {name:'Save'}))
    await waitFor(() => expect(vi.mocked(api).mock.calls.some(([,path,opts]) => path === '/bugs/demo-1' && opts?.method === 'PATCH' && JSON.parse(opts.body as string).note === '')).toBe(true))
  })
})
describe('research input snapshot', () => {
  it('renders the ticket snapshot as a readable report, not serialized JSON', async () => {
    const snapshot = JSON.stringify({id:'demo-1', project:'demo', type:'bug', title:'Receipt crashes', body:'Crashes on **empty** receipts.\n\n## Steps to reproduce\nImport an empty file.', state:'todo', priority:'high', version:3, created_by:'ada', created_at:'2026-09-01T10:00:00Z', claim:null, metadata:{internal:true}, occurrence:null})
    jobs = [{id:'aj-1', status:'completed', created_at:1788690000000, response:'Findings', prompt:'Check imports', snapshot}]
    mount()
    await screen.findByRole('button', {name:'Research again'})
    const view = screen.getByTestId('snapshot')
    expect(view.textContent).toContain('Title: Receipt crashes')
    expect(view.textContent).toContain('Ticket status: todo')
    expect(view.textContent).toContain('Priority: high')
    expect(view.textContent).toContain('Ticket version: 3')
    expect(view.textContent).toContain('Reported by: ada')
    expect(screen.getByText('Steps to reproduce').tagName).toMatch(/^H[1-6]$/)
    expect(screen.getByText('empty').tagName).toMatch(/^(STRONG|B)$/)
    expect(view.textContent).not.toContain('{"id"')
    expect(view.textContent).not.toContain('metadata')
    expect(view.textContent).not.toContain('occurrence')
    expect(view.textContent).not.toContain('\\n')
  })
})
