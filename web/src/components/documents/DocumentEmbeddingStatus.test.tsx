import { ProjectUpdatesContext, type ProjectUpdate } from '@/hooks/useProjectUpdates'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { EmbeddingStatusProvider, STATUS_POLL_IDLE_MS, STATUS_POLL_OPEN_MS } from '@/hooks/useEmbeddingStatus'
import type { ServerSync } from '@/lib/save-status'
import { DocumentEmbeddingStatus } from './DocumentEmbeddingStatus'
import { DocumentHybridSearch } from './DocumentHybridSearch'
import type { SearchStatus } from '@/lib/hybrid-search'
const current: SearchStatus = { configured: true, indexed: 2, total: 2, passages_indexed: 7, passages_total: 7, queued: 0, pending: 0, running: 0, failed: 0, last_error: null, projection: 'current', last_synced_at: 1750000000000 }
const reply = (value: unknown) => new Response(JSON.stringify(value))
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); visibility('visible') })
function visibility(value: 'visible' | 'hidden') {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => value })
  document.dispatchEvent(new Event('visibilitychange'))
}
const tick = (ms = 0) => act(async () => { await vi.advanceTimersByTimeAsync(ms) })
function Fixture({ pending = false, server = pending ? 'behind' : 'current', canSync = true, search = false }: { pending?: boolean; server?: ServerSync; canSync?: boolean; search?: boolean }) {
  return <EmbeddingStatusProvider token="test" map="map" server={server}><DocumentEmbeddingStatus locale="en" canSync={canSync} />{search && <DocumentHybridSearch token="test" map="map" locale="en" canSync={canSync} onNavigate={() => {}} />}</EmbeddingStatusProvider>
}
describe('document embedding status', () => {
  it('shows passage counts separately from nonoverlapping section jobs and keeps historical sync time', async () => {
    const status = { ...current, passages_indexed: 7, passages_total: 11, queued: 6, pending: 3, running: 2, failed: 1 }
    vi.stubGlobal('fetch', vi.fn(async () => reply(status)))
    render(<Fixture />)
    fireEvent.click(await screen.findByRole('button', { name: 'Embedding error' }))
    expect(screen.getByText('7 / 11')).toBeTruthy()
    for (const [label, value] of [['Sections pending', '3'], ['Sections running', '2'], ['Sections failed', '1']]) {
      expect(screen.getByText(label!).nextElementSibling?.textContent).toBe(value)
    }
    expect(document.querySelector('time')?.getAttribute('datetime')).toBe(new Date(current.last_synced_at!).toISOString())
    expect(screen.getByRole('button', { name: 'Embed now' })).toBeTruthy()
  })
  it('uses one reader for ribbon and search, and returns keyboard focus on Escape', async () => {
    const fetch = vi.fn(async (_url: string) => reply(current)); vi.stubGlobal('fetch', fetch)
    render(<Fixture search />)
    const button = await screen.findByRole('button', { name: 'Embeddings current' })
    button.focus(); fireEvent.click(button)
    expect(screen.getByRole('dialog', { name: 'Document embeddings' })).toBeTruthy()
    fireEvent.keyDown(screen.getByRole('button', { name: 'Close' }), { key: 'Escape' })
    await waitFor(() => expect(document.activeElement).toBe(button))
    fireEvent.click(screen.getByRole('button', { name: /Search document/ }))
    await screen.findByRole('combobox')
    await waitFor(() => expect(fetch.mock.calls).toHaveLength(3))
    expect(new Set(fetch.mock.calls.map(([url]) => String(url))).size).toBe(1)
    expect(screen.getByRole('status').textContent).toMatch(/^Index current/)
  })
  it('withholds completion until a fresh read after local edits are saved', async () => {
    let resolveFresh!: (value: Response) => void
    let requests = 0
    vi.stubGlobal('fetch', vi.fn(async () => ++requests < 3 ? reply(current) : new Promise<Response>(resolve => { resolveFresh = resolve })))
    const view = render(<Fixture />)
    await screen.findByRole('button', { name: 'Embeddings current' })
    view.rerender(<Fixture pending />)
    const pending = await screen.findByRole('button', { name: 'Embeddings pending' })
    fireEvent.click(pending)
    expect((screen.getByRole('button', { name: 'Embed now' }) as HTMLButtonElement).disabled).toBe(true)
    await waitFor(() => expect(requests).toBe(2))
    view.rerender(<Fixture />)
    expect(screen.queryByRole('button', { name: 'Embeddings current' })).toBeNull()
    await waitFor(() => expect(resolveFresh).toBeTypeOf('function'))
    await act(async () => resolveFresh(reply({ ...current, queued: 1, pending: 1 })))
    expect(screen.getByRole('status').textContent).toBe('Embeddings pending')
    expect(document.querySelector('time')?.getAttribute('datetime')).toBe(new Date(current.last_synced_at!).toISOString())
  })
  it('does not let an older manual-sync response mark newer local edits complete', async () => {
    let finishSync!: (value: Response) => void
    vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => init?.method === 'POST'
      ? new Promise<Response>(resolve => { finishSync = resolve }) : reply(current)))
    const view = render(<Fixture />)
    fireEvent.click(await screen.findByRole('button', { name: 'Embeddings current' }))
    fireEvent.click(screen.getByRole('button', { name: 'Embed now' }))
    await waitFor(() => expect(finishSync).toBeTypeOf('function'))
    view.rerender(<Fixture pending />)
    view.rerender(<Fixture />)
    await act(async () => finishSync(reply({ ...current, sync: 'scheduled' })))
    expect(screen.getByRole('status').textContent).toBe('Checking embeddings…')
  })
  it('honors deferred Embed now and keeps readers from scheduling', async () => {
    const fetch = vi.fn(async (_url: string, init?: RequestInit) => reply(init?.method === 'POST' ? { ...current, projection: 'stale', sync: 'deferred' } : current))
    vi.stubGlobal('fetch', fetch)
    const view = render(<Fixture />)
    fireEvent.click(await screen.findByRole('button', { name: 'Embeddings current' }))
    fireEvent.click(screen.getByRole('button', { name: 'Embed now' }))
    await screen.findByText(/Nothing was scheduled/)
    expect(fetch.mock.calls.filter(([, init]) => init?.method === 'POST')).toHaveLength(1)
    view.unmount()
    render(<Fixture canSync={false} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Embeddings current' }))
    expect(screen.queryByRole('button', { name: 'Embed now' })).toBeNull()
  })
  it('shows unconfigured and no recorded sync without inventing a timestamp', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => reply({ ...current, configured: false, passages_indexed: 0, last_synced_at: null })))
    render(<Fixture />)
    fireEvent.click(await screen.findByRole('button', { name: 'Embeddings not configured' }))
    expect(screen.getByText('Not yet recorded')).toBeTruthy()
    expect(document.querySelector('time')).toBeNull()
    expect(screen.getByRole('link', { name: 'Configure meaning search' })).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Embed now' }) as HTMLButtonElement).disabled).toBe(true)
  })
  it('trusts the server acknowledgement rather than the local replica for pending', async () => {
    let requests = 0
    vi.stubGlobal('fetch', vi.fn(async () => { requests++; return reply(current) }))
    const view = render(<Fixture server="unknown" />)
    await waitFor(() => expect(requests).toBe(1))
    expect(screen.getByRole('button', { name: 'Checking embeddings…' })).toBeTruthy()
    view.rerender(<Fixture server="current" />)
    const button = await screen.findByRole('button', { name: 'Embeddings current' })
    expect(requests).toBe(1)
    fireEvent.click(button)
    expect((screen.getByRole('button', { name: 'Embed now' }) as HTMLButtonElement).disabled).toBe(false)
    expect(screen.queryByText(/have not reached the server/)).toBeNull()
    view.rerender(<Fixture server="behind" />)
    expect(screen.getByText('Embeddings pending')).toBeTruthy()
    expect(screen.getByText(/have not reached the server/)).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Embed now' }) as HTMLButtonElement).disabled).toBe(true)
  })
  it('polls slowly while closed, every few seconds while a dialog is open, and only on return while hidden', async () => {
    vi.useFakeTimers()
    let hold: ((value: Response) => void) | null = null
    const fetch = vi.fn(() => hold ? new Promise<Response>(resolve => { hold = resolve }) : Promise.resolve(reply(current)))
    vi.stubGlobal('fetch', fetch)
    render(<Fixture />)
    await tick()
    expect(fetch).toHaveBeenCalledTimes(1)
    await tick(STATUS_POLL_IDLE_MS - 1)
    expect(fetch).toHaveBeenCalledTimes(1)
    await tick(1)
    expect(fetch).toHaveBeenCalledTimes(2)
    hold = () => {}
    fireEvent.click(screen.getByRole('button', { name: 'Embeddings current' }))
    await tick()
    expect(fetch).toHaveBeenCalledTimes(3)
    expect(screen.getByRole('status').textContent).toBe('Checking embeddings…')
    await act(async () => { hold!(reply(current)); hold = null })
    expect(screen.getByRole('status').textContent).toBe('Embeddings current')
    await tick(STATUS_POLL_OPEN_MS)
    expect(fetch).toHaveBeenCalledTimes(4)
    act(() => visibility('hidden'))
    await tick(STATUS_POLL_IDLE_MS * 3)
    expect(fetch).toHaveBeenCalledTimes(4)
    act(() => visibility('visible'))
    await tick()
    expect(fetch).toHaveBeenCalledTimes(5)
    fireEvent.keyDown(screen.getByRole('button', { name: 'Close' }), { key: 'Escape' })
    await tick(STATUS_POLL_OPEN_MS)
    expect(fetch).toHaveBeenCalledTimes(6)
    await tick(STATUS_POLL_OPEN_MS)
    expect(fetch).toHaveBeenCalledTimes(6)
    await tick(STATUS_POLL_IDLE_MS - STATUS_POLL_OPEN_MS)
    expect(fetch).toHaveBeenCalledTimes(7)
  })
  it('reads once when edits are saved, not when they become pending, and keeps the idle cadence meanwhile', async () => {
    vi.useFakeTimers()
    const fetch = vi.fn(async () => reply(current)); vi.stubGlobal('fetch', fetch)
    const view = render(<Fixture server="current" />)
    await tick()
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: 'Embeddings current' })).toBeTruthy()
    for (let pause = 0; pause < 3; pause++) {
      view.rerender(<Fixture server="behind" />)
      await tick(1000)
      expect(fetch).toHaveBeenCalledTimes(1 + pause)
      expect(screen.getByRole('button', { name: 'Embeddings pending' })).toBeTruthy()
      view.rerender(<Fixture server="current" />)
      await tick(1000)
      expect(fetch).toHaveBeenCalledTimes(2 + pause)
      expect(screen.getByRole('button', { name: 'Embeddings current' })).toBeTruthy()
    }
    await tick(STATUS_POLL_IDLE_MS - 1000)
    expect(fetch).toHaveBeenCalledTimes(5)
    await tick(1000)
    expect(fetch).toHaveBeenCalledTimes(5)
  })
  it('ignores a late aborted completion without arming a timer over its replacement', async () => {
    vi.useFakeTimers()
    const pending: Array<(value: Response) => void> = []
    const fetch = vi.fn((_url: string, _init?: RequestInit) => new Promise<Response>(resolve => pending.push(resolve)))
    vi.stubGlobal('fetch', fetch)
    render(<Fixture />)
    await tick()
    expect(fetch).toHaveBeenCalledTimes(1)
    fireEvent.click(screen.getByRole('button', { name: 'Checking embeddings…' }))
    await tick()
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(fetch.mock.calls[0]![1]?.signal?.aborted).toBe(true)
    // The mock deliberately ignores abort, just like a response already parsing.
    await act(async () => pending[0]!(reply(current)))
    await tick(STATUS_POLL_OPEN_MS * 2)
    expect(fetch).toHaveBeenCalledTimes(2)
    expect(screen.getByRole('status').textContent).toBe('Checking embeddings…')
    await act(async () => pending[1]!(reply({ ...current, queued: 1, pending: 1 })))
    expect(screen.getByRole('status').textContent).toBe('Embeddings pending')
    await tick(STATUS_POLL_OPEN_MS - 1)
    expect(fetch).toHaveBeenCalledTimes(2)
    await tick(1)
    expect(fetch).toHaveBeenCalledTimes(3)
  })
  it('resumes polling after manual sync aborts an in-flight status read', async () => {
    vi.useFakeTimers()
    let reads = 0
    const fetch = vi.fn((_url: string, init?: RequestInit) => {
      if (init?.method === 'POST') return Promise.resolve(reply({ ...current, queued: 1, pending: 1, sync: 'scheduled' }))
      reads++
      return reads === 2 ? new Promise<Response>(() => {}) : Promise.resolve(reply(current))
    })
    vi.stubGlobal('fetch', fetch)
    render(<Fixture />)
    await tick()
    fireEvent.click(screen.getByRole('button', { name: 'Embeddings current' }))
    await tick()
    fireEvent.click(screen.getByRole('button', { name: 'Embed now' }))
    await tick()
    expect(screen.getByRole('status').textContent).toBe('Embeddings pending')
    await tick(STATUS_POLL_OPEN_MS)
    expect(reads).toBe(3)
    expect(screen.getByRole('status').textContent).toBe('Embeddings current')
  })

})

it('refreshes lexical completion from a search event without healthy idle polling', async () => {
  vi.useFakeTimers()
  let listener: ((event?: ProjectUpdate) => Promise<unknown>) | undefined
  const fetch = vi.fn(async () => reply({ ...current, configured: false, projection: 'pending' }))
  vi.stubGlobal('fetch', fetch)
  const shared = { project: 'demo', connected: true, subscribe: (callback: (event?: ProjectUpdate) => Promise<unknown>) => { listener = callback; return () => { listener = undefined } } }
  render(<ProjectUpdatesContext value={shared}><Fixture /></ProjectUpdatesContext>)
  await tick()
  expect(fetch).toHaveBeenCalledTimes(1)
  await tick(60000)
  expect(fetch).toHaveBeenCalledTimes(1)
  fetch.mockImplementation(async () => reply({ ...current, configured: false }))
  await act(async () => listener?.({ type: 'refresh', topics: ['search'] }))
  await tick(1000)
  expect(fetch).toHaveBeenCalledTimes(2)
  await tick(60000)
  expect(fetch).toHaveBeenCalledTimes(2)
})
