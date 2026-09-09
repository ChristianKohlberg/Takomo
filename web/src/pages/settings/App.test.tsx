import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { createMemoryRouter, RouterProvider } from 'react-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ToastProvider } from '@/components/Toaster'
import { App } from './App'
let scopes: string[], failSave: boolean, failDelete: boolean, style: string, deleted: string[], holdProjects: Promise<void> | null
const fetcher = vi.fn(async (url: string, opts?: RequestInit) => {
  if (opts?.method === 'DELETE') {
    if (failDelete) return new Response(JSON.stringify({ message: 'Delete refused' }), { status: 409 })
    deleted.push(url); return new Response('{}')
  }
  if (url === '/v1/projects' && holdProjects) await holdProjects
  if (opts?.method === 'PUT') {
    if (failSave) return new Response(JSON.stringify({ message: 'Save refused' }), { status: 422 })
    if (url.endsWith('/style')) style = JSON.parse(String(opts.body)).style_guide
    return new Response('{}')
  }
  if (url.endsWith('/workflow')) return new Response('{}', { status: 404 })
  const data = url === '/v1/whoami' ? { actor: 'test:admin', scopes, token_id: 'test', projects: null }
    : url === '/v1/projects' ? [{ id: 'takomo', name: 'Takomo', style_guide: style }, { id: 'second', name: 'Second project' }].filter(p => !deleted.includes(`/v1/projects/${p.id}`))
    : url.includes('/users') ? { items: [], total: 0 }
    : url.endsWith('/writing-instructions') ? { templates: [], default_id: null }
    : url.endsWith('/document-classification-policy') ? { mode: 'suggest' } : []
  return new Response(JSON.stringify(data), { headers: { 'Content-Type': 'application/json' } })
})
function open(path: string) {
  const router = createMemoryRouter([{ path: '/settings', element: <App /> }, { path: '/legacy', element: <App legacy /> }], { initialEntries: [path] })
  render(<ToastProvider><RouterProvider router={router} /></ToastProvider>)
  return router
}
beforeEach(() => {
  localStorage.clear(); localStorage.setItem('takomo.token', 'test-only'); localStorage.setItem('takomo.lang', 'en')
  scopes = ['read', 'write', 'human', 'admin']; failSave = false; failDelete = false; style = 'Original style'; deleted = []; holdProjects = null
  fetcher.mockClear(); vi.stubGlobal('fetch', fetcher)
})
describe('Settings navigation and save boundaries', () => {
  it('opens old project links and moves secondary workspaces out of Settings', async () => {
    open('/settings?project=takomo')
    await screen.findByText('Project name')
    expect(screen.getByText('Project ID')).toBeTruthy()
    expect(screen.queryByRole('link', { name: /Agent queue/ })).toBeNull()
    expect(screen.getByRole('link', { name: 'Legacy' }).getAttribute('href')).toBe('/legacy?scope=takomo')
    expect(screen.getByRole('link', { name: 'General' }).getAttribute('aria-current')).toBe('page')
  })
  it('honors section URLs and Back, separating writing from appearance and timing', async () => {
    const router = open('/settings?scope=takomo&section=writing')
    await screen.findByLabelText('Style guide')
    expect(screen.queryByLabelText(/Answer-link lifetime/)).toBeNull()
    expect(screen.queryByLabelText('Template')).toBeNull()
    fireEvent.click(screen.getByRole('link', { name: 'Document appearance' }))
    await screen.findByLabelText('Template')
    expect(screen.queryByLabelText('Style guide')).toBeNull()
    await act(() => router.navigate(-1))
    await screen.findByLabelText('Style guide')
  })
  it('keeps drafts on Stay and discards only after confirmation', async () => {
    const router = open('/settings?scope=takomo&section=writing')
    fireEvent.change(await screen.findByLabelText('Style guide'), { target: { value: 'Unsaved draft' } })
    fireEvent.click(screen.getByRole('link', { name: 'People' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Stay here' }))
    expect((screen.getByLabelText('Style guide') as HTMLTextAreaElement).value).toBe('Unsaved draft')
    fireEvent.click(screen.getByRole('link', { name: 'People' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Discard and continue' }))
    await waitFor(() => expect(router.state.location.search).toContain('section=people'))
    await act(() => router.navigate(-1))
    expect((await screen.findByLabelText('Style guide') as HTMLTextAreaElement).value).toBe('Original style')
  })
  it('saves only changed conventions and preserves rejected drafts', async () => {
    open('/settings?scope=takomo&section=writing')
    fireEvent.change(await screen.findByLabelText('Style guide'), { target: { value: 'New style' } })
    failSave = true
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await screen.findByText('Save refused')
    expect((screen.getByLabelText('Style guide') as HTMLTextAreaElement).value).toBe('New style')
    failSave = false
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await screen.findByText('Saved.')
    await waitFor(() => expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true))
    const writes = fetcher.mock.calls.filter(([, opts]) => opts?.method === 'PUT')
    expect(writes.map(([url]) => url)).toEqual(['/v1/projects/takomo/style', '/v1/projects/takomo/style'])
    expect(JSON.parse(String(writes[1]?.[1]?.body))).toEqual({ style_guide: 'New style' })
  })
  it('keeps instance pages independent of the selected project', async () => {
    const router = open('/settings?section=people&scope=takomo')
    await screen.findByRole('button', { name: /Add person/ })
    fireEvent.change(screen.getByLabelText('Project'), { target: { value: 'second' } })
    await waitFor(() => expect(router.state.location.search).toBe('?section=people&scope=second'))
    expect(screen.getByText('Instance-wide')).toBeTruthy()
  })
  it('switches sections without refetching the admin lists', async () => {
    const reads = () => fetcher.mock.calls.filter(([, opts]) => !opts?.method || opts.method === 'GET').map(([url]) => url)
    open('/settings?scope=takomo&section=writing')
    await screen.findByLabelText('Style guide')
    await waitFor(() => expect(reads()).toContain('/v1/tokens'))
    const loaded = reads()
    fireEvent.click(screen.getByRole('link', { name: 'People' }))
    await screen.findByRole('button', { name: /Add person/ })
    fireEvent.click(screen.getByRole('link', { name: 'Workflow & timing' }))
    await screen.findByLabelText(/Answer-link lifetime/)
    expect(reads()).toEqual(loaded)
  })
  it('lets the blank project choice clear the remembered project', async () => {
    const router = open('/settings?section=people&scope=takomo')
    await screen.findByRole('button', { name: /Add person/ })
    fireEvent.change(screen.getByLabelText('Project'), { target: { value: '' } })
    await waitFor(() => expect(router.state.location.search).toBe('?section=people'))
    expect(localStorage.getItem('takomo.project')).toBe('')
    await waitFor(() => expect((screen.getByLabelText('Project') as HTMLSelectElement).value).toBe(''))
    fireEvent.click(screen.getByRole('link', { name: 'General' }))
    await screen.findByText('Select a project in the navigation.')
  })
  it('drops the scope of a project deleted from its own General page', async () => {
    const router = open('/settings?scope=takomo&section=general')
    await screen.findByText('Project name')
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Delete project' }))
    await waitFor(() => expect(deleted).toEqual(['/v1/projects/takomo']))
    await waitFor(() => expect(router.state.location.search).toBe('?section=projects'))
    expect(localStorage.getItem('takomo.project')).toBe('')
    await waitFor(() => expect(screen.queryByText('Takomo')).toBeNull())
    await waitFor(() => expect((screen.getByLabelText('Project') as HTMLSelectElement).value).toBe(''))
    expect(screen.getByRole('option', { name: 'Select project' })).toBeTruthy()
    expect(screen.queryByRole('option', { name: 'takomo' })).toBeNull()
  })
  it('keeps the delete dialog busy and single-shot while the refresh is still in flight', async () => {
    const router = open('/settings?scope=takomo&section=general')
    await screen.findByText('Project name')
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    let release = () => {}
    holdProjects = new Promise<void>(resolve => { release = resolve })
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Delete project' }))
    await waitFor(() => expect(deleted).toEqual(['/v1/projects/takomo']))
    await waitFor(() => expect(router.state.location.search).toBe('?section=projects'))
    const confirm = screen.getByRole('button', { name: 'Delete project' }) as HTMLButtonElement
    expect(confirm.disabled).toBe(true)
    fireEvent.click(confirm)
    await act(async () => { await Promise.resolve() })
    expect(deleted).toEqual(['/v1/projects/takomo'])
    expect(screen.getAllByRole('dialog')).toHaveLength(1)
    release()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(deleted).toEqual(['/v1/projects/takomo'])
  })
  it('deletes a project with an unsaved classification draft without asking about it', async () => {
    const router = open('/settings?scope=takomo&section=general')
    const policy = await screen.findByRole('combobox', { name: 'Document classification' })
    await waitFor(() => expect((policy as HTMLSelectElement).disabled).toBe(false))
    fireEvent.change(policy, { target: { value: 'auto_apply_clear' } })
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Delete project' }))
    await waitFor(() => expect(router.state.location.search).toBe('?section=projects'))
    expect(screen.queryByText('Discard unsaved changes?')).toBeNull()
    expect(localStorage.getItem('takomo.project')).toBe('')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(deleted).toEqual(['/v1/projects/takomo'])
  })
  it('keeps guarding an unsaved classification draft when deletion fails', async () => {
    failDelete = true
    const router = open('/settings?scope=takomo&section=general')
    const policy = await screen.findByRole('combobox', { name: 'Document classification' })
    await waitFor(() => expect((policy as HTMLSelectElement).disabled).toBe(false))
    fireEvent.change(policy, { target: { value: 'auto_apply_clear' } })
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Delete project' }))
    await screen.findByText('Delete refused')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    fireEvent.click(screen.getByRole('link', { name: 'People' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Stay here' }))
    expect(router.state.location.search).toBe('?scope=takomo&section=general')
    expect((screen.getByRole('combobox', { name: 'Document classification' }) as HTMLSelectElement).value).toBe('auto_apply_clear')
    expect(deleted).toEqual([])
  })
  it('allows non-admins into Legacy but preserves administrative permissions', async () => {
    scopes = ['read']; open('/legacy?scope=takomo')
    expect((await screen.findByRole('link', { name: /Agent queue/ })).getAttribute('href')).toBe('/agent-queues?project=takomo')
    fireEvent.click(screen.getByRole('link', { name: 'API tokens' }))
    await screen.findByText('This token is not an admin')
    expect(screen.queryByRole('button', { name: /New token/ })).toBeNull()
  })
})
