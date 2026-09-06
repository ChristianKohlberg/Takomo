import type { ReactNode } from 'react'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { App } from './App'
import { useSpecification } from './context'

const api = vi.hoisted(() => ({
  whoami: vi.fn(),
  listProjects: vi.fn(),
  listMindmaps: vi.fn(),
  createMindmap: vi.fn(),
  mintMindmapSession: vi.fn(),
  toast: vi.fn(),
}))
vi.mock('@/lib/initiatives', async (original) => ({
  ...(await original<typeof import('@/lib/initiatives')>()),
  whoami: api.whoami,
  listProjects: api.listProjects,
}))
vi.mock('@/lib/mindmaps', async (original) => ({
  ...(await original<typeof import('@/lib/mindmaps')>()),
  listMindmaps: api.listMindmaps,
  createMindmap: api.createMindmap,
  mintMindmapSession: api.mintMindmapSession,
}))
vi.mock('@/lib/verification', async (original) => ({
  ...(await original<typeof import('@/lib/verification')>()),
  listChecks: vi.fn(async () => ({ items: [] })),
}))
vi.mock('@/lib/test-runs', async (original) => ({
  ...(await original<typeof import('@/lib/test-runs')>()),
  listDefinitions: vi.fn(async () => []),
}))
vi.mock('@/hooks/useProjectUpdates', async (original) => ({
  ...(await original<typeof import('@/hooks/useProjectUpdates')>()),
  useProjectUpdates: vi.fn(),
}))
vi.mock('@/hooks/useSyncConnection', () => ({
  useSyncConnection: (session: unknown) => (session ? connection : null),
}))
vi.mock('@/components/Toaster', () => ({ useToast: () => ({ toast: api.toast }) }))
vi.mock('@/components/AppShell', () => ({
  AppShell: ({ children }: { children: ReactNode }) => <>{children}</>,
}))
vi.mock('@/components/AppHeader', () => ({
  AppHeader: ({ children, views }: { children: ReactNode; views: ReactNode }) => (
    <header>
      {views}
      {children}
    </header>
  ),
}))
vi.mock('./History', () => ({ default: () => null }))
vi.mock('../documents/App', () => ({ DocumentView: () => <View name="Document" /> }))
vi.mock('../mindmap/App', () => ({ MapView: () => <View name="Map" /> }))
vi.mock('../verification/App', () => ({ TestsView: () => <View name="Tests" /> }))

function View({ name }: { name: string }) {
  const { map, session, nodes } = useSpecification()
  return (
    <main aria-label={`${name} editor`} data-map={map?.id} data-session={session?.mindmap}>
      {nodes.length} sections
    </main>
  )
}
const provider = {
  awareness: {
    clientID: 1,
    getStates: () => new Map(),
    on: vi.fn(),
    off: vi.fn(),
    setLocalStateField: vi.fn(),
  },
  connect: vi.fn(),
}
let connection = { ydoc: new Y.Doc(), provider }
const map = {
  id: 'mm-project',
  project: 'vetbill',
  title: 'VetBill',
  status: 'open',
  nodes: 0,
  created_at: '',
}

function mount(view = 'document') {
  return render(
    <MemoryRouter initialEntries={[`/projects/vetbill/specification?view=${view}`]}>
      <App />
    </MemoryRouter>,
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  localStorage.setItem('takomo.token', 'token')
  localStorage.setItem('takomo.lang', 'en')
  connection = { ydoc: new Y.Doc(), provider }
  api.whoami.mockResolvedValue({ actor: 'writer', scopes: ['read', 'write'] })
  api.listProjects.mockResolvedValue([{ id: 'vetbill', name: 'VetBill' }])
  api.listMindmaps.mockResolvedValue({ items: [] })
  api.createMindmap.mockResolvedValue({ mindmap: map })
  api.mintMindmapSession.mockResolvedValue({
    object: map.id,
    mindmap: map.id,
    kind: 'mindmap',
    session: 'session',
    token: 'ticket',
    can_write: true,
    display: 'Writer',
    expires_at: '',
    url: '/sync',
    room: map.id,
  })
})
afterEach(() => {
  cleanup()
  connection.ydoc.destroy()
})

describe('opening a project specification', () => {
  it('automatically opens an empty document and the same map without a create dialog', async () => {
    mount()
    const document = await screen.findByRole('main', { name: 'Document editor' })
    expect(document.textContent).toBe('0 sections')
    expect(document.getAttribute('data-map')).toBe(map.id)
    expect(document.getAttribute('data-session')).toBe(map.id)
    expect(api.createMindmap).toHaveBeenCalledExactlyOnceWith('token', {
      project: 'vetbill',
      title: 'VetBill',
    })
    expect(screen.queryByRole('button', { name: 'Create plan' })).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByRole('link', { name: 'Map' }))
    const canvas = await screen.findByRole('main', { name: 'Map editor' })
    expect(canvas.getAttribute('data-map')).toBe(map.id)
    expect(canvas.getAttribute('data-session')).toBe(map.id)
    expect(canvas.textContent).toBe('0 sections')
    expect(api.createMindmap).toHaveBeenCalledOnce()
  })

  it('waits for the session before mounting an editor', async () => {
    let resolve!: (session: unknown) => void
    api.mintMindmapSession.mockReturnValue(
      new Promise((done) => {
        resolve = done
      }),
    )
    mount('map')
    await waitFor(() => expect(api.mintMindmapSession).toHaveBeenCalled())
    expect(screen.queryByRole('main', { name: 'Map editor' })).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()
    resolve({ mindmap: map.id, display: 'Writer' })
    await screen.findByRole('main', { name: 'Map editor' })
  })

  it.each([['read', 'write'], ['read']])(
    'opens an existing map with scopes %j without replacing it',
    async (...scopes) => {
      api.whoami.mockResolvedValue({ actor: 'viewer', scopes })
      const existing = { ...map, id: 'mm-existing', title: 'Existing requirements' }
      api.listMindmaps.mockResolvedValue({ items: [existing] })
      api.mintMindmapSession.mockResolvedValue({ mindmap: existing.id, display: 'Writer' })
      mount()
      const editor = await screen.findByRole('main', { name: 'Document editor' })
      expect(editor.getAttribute('data-map')).toBe(existing.id)
      expect(api.mintMindmapSession).toHaveBeenCalledWith('token', existing.id)
      expect(api.createMindmap).not.toHaveBeenCalled()
    },
  )

  it('opens an archived project without a map as read-only instead of loading forever', async () => {
    api.listProjects.mockResolvedValue([{ id: 'vetbill', name: 'VetBill', archived: true }])
    mount()
    await screen.findByText(/This project is archived/)
    expect(api.listMindmaps).toHaveBeenCalled()
    expect(api.createMindmap).not.toHaveBeenCalled()
    expect(api.toast).not.toHaveBeenCalled()
  })

  it('treats an archive refusal on first visit as read-only rather than an error', async () => {
    api.createMindmap.mockRejectedValue(Object.assign(new Error('frozen'), { status: 409, code: 'project.archived' }))
    mount()
    await screen.findByText(/This project is archived/)
    expect(api.createMindmap).toHaveBeenCalledOnce()
    expect(api.mintMindmapSession).not.toHaveBeenCalled()
    expect(api.toast).not.toHaveBeenCalled()
    expect(screen.queryByText('Loading specification…')).toBeNull()
  })

  it.each([
    ['a write budget refusal', { status: 429, code: 'rate.limited', message: 'Write budget exhausted' }],
    ['a stale project id', { status: 404, code: 'project.not_found', message: "Project 'vetbill' not found" }],
  ])('ends the spinner on %s with the actual error and a retry that recovers', async (_, refusal) => {
    api.createMindmap.mockRejectedValueOnce(Object.assign(new Error(refusal.message), refusal))
    mount()
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toContain(refusal.message)
    expect(screen.queryByText('Loading specification…')).toBeNull()
    expect(screen.queryByText(/This project is archived/)).toBeNull()
    expect(screen.queryByRole('main', { name: 'Document editor' })).toBeNull()
    expect(api.createMindmap).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    const document = await screen.findByRole('main', { name: 'Document editor' })
    expect(document.getAttribute('data-map')).toBe(map.id)
    expect(screen.queryByRole('alert')).toBeNull()
    expect(api.createMindmap).toHaveBeenCalledTimes(2)
  })

  it('does not ask a reader to create a missing specification', async () => {
    api.whoami.mockResolvedValue({ actor: 'reader', scopes: ['read'] })
    mount()
    await screen.findByText(/Your token can read this plan/)
    expect(api.listMindmaps).toHaveBeenCalled()
    expect(api.createMindmap).not.toHaveBeenCalled()
    expect(api.mintMindmapSession).not.toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: 'Create plan' })).toBeNull()
  })
})
