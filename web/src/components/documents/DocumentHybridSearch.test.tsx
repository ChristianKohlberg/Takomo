import { Activity } from 'react'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DocumentHybridSearch, SearchExcerpt } from './DocumentHybridSearch'
import type { SearchResponse, SearchResult } from '@/lib/hybrid-search'
const one: SearchResult = { node_id: 'one', title: 'Delivery', heading_path: ['Operations'], excerpt: 'parcel <script>alert(1)</script>', passage: 'parcel', highlights: ['parcel'], match_kind: 'keyword' }
const two: SearchResult = { ...one, node_id: 'two', title: 'Shipping', match_kind: 'semantic', highlights: [] }
const response: SearchResponse = { results: [one, two], limit: 20, candidates: 2, truncated: false, mode: 'keyword', semantic_status: 'unconfigured', projection: 'current', projection_error: null }
function mockFetch(search: (url: string, init?: RequestInit) => Promise<Response> = async () => new Response(JSON.stringify(response))) {
  vi.stubGlobal('fetch', vi.fn((url: string, init?: RequestInit) => url.endsWith('/status') ? Promise.resolve(new Response(JSON.stringify({ configured: false, queued: 0, running: 0, failed: 0, indexed: 0, total: 2, last_error: null, projection: 'current' }))) : search(url, init)))
}
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers() })
describe('document hybrid search', () => {
  it('escapes excerpts and never highlights semantic-only results', () => {
    const { container } = render(<SearchExcerpt result={one} />)
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('mark')?.textContent).toBe('parcel')
    cleanup()
    const semantic = render(<SearchExcerpt result={{ ...two, highlights: ['parcel'] }} />)
    expect(semantic.container.querySelector('mark')).toBeNull()
  })
  it('opens with save shortcut, navigates with arrows/Enter and restores focus on Escape', async () => {
    mockFetch()
    const navigate = vi.fn()
    render(<><button>Editor stand-in</button><DocumentHybridSearch token="test" map="m" locale="en" canSync onNavigate={navigate} /></>)
    const editor = screen.getByText('Editor stand-in'); editor.focus()
    fireEvent.keyDown(window, { key: 's', ctrlKey: true })
    const input = await screen.findByRole('combobox')
    expect(document.activeElement).toBe(input)
    fireEvent.change(input, { target: { value: 'parcel' } })
    await screen.findByText('Shipping')
    expect(screen.getByText('Related meaning')).toBeTruthy()
    expect((screen.getByText('Sync document') as HTMLButtonElement).disabled).toBe(true)
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(navigate).toHaveBeenCalledWith(two))
    editor.focus(); fireEvent.keyDown(window, { key: 's', metaKey: true })
    fireEvent.keyDown(await screen.findByRole('combobox'), { key: 'Escape' })
    await waitFor(() => expect(document.activeElement).toBe(editor))
  }, 15000)
  it('releases the save shortcut when Document is an inactive specification tab', () => {
    mockFetch()
    const view = render(<Activity mode="visible"><DocumentHybridSearch token="test" map="m" locale="en" canSync={false} onNavigate={vi.fn()} /></Activity>)
    view.rerender(<Activity mode="hidden"><DocumentHybridSearch token="test" map="m" locale="en" canSync={false} onNavigate={vi.fn()} /></Activity>)
    const shortcut = new KeyboardEvent('keydown', { key: 's', ctrlKey: true, cancelable: true })
    window.dispatchEvent(shortcut)
    expect(shortcut.defaultPrevented).toBe(false)
    expect(screen.queryByRole('dialog')).toBeNull()
  })
  it('schedules a manual sync and announces running status without blocking keyword results', async () => {
    const fetch = vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.endsWith('/sync')
      ? { configured: true, queued: 0, running: 1, failed: 0, indexed: 1, total: 2, last_error: null, projection: 'current' }
      : url.endsWith('/status') ? { configured: true, queued: 1, running: 0, failed: 0, indexed: 1, total: 2, last_error: null, projection: 'current' }
      : response))))
    vi.stubGlobal('fetch', fetch)
    render(<DocumentHybridSearch token="test" map="m" locale="en" canSync onNavigate={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Search document/ }))
    await screen.findByText(/Update pending/)
    fireEvent.click(screen.getByText('Sync document'))
    await screen.findByText(/Updating index/)
    const call = fetch.mock.calls.find(([url]) => url.endsWith('/sync'))
    expect(call).toBeTruthy()
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'parcel' } })
    await screen.findByText('Shipping')
  })
  it('distinguishes a truncated top list, a throttled semantic pass and parked jobs', async () => {
    vi.stubGlobal('fetch', vi.fn((url: string) => Promise.resolve(new Response(JSON.stringify(url.endsWith('/status')
      ? { configured: true, queued: 3, running: 0, failed: 3, indexed: 5, total: 8, last_error: 'Embedding provider returned HTTP 400', projection: 'stale' }
      : { ...response, candidates: 37, truncated: true, note: 'Showing the 2 best-ranked of 37 candidate sections.', semantic_status: 'throttled', projection: 'stale', projection_error: 'Cannot decode source document' })))))
    render(<DocumentHybridSearch token="test" map="m" locale="en" canSync onNavigate={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Search document/ }))
    await screen.findByText(/Indexing gave up on 3 sections/)
    expect((screen.getByText('Sync document') as HTMLButtonElement).disabled).toBe(false)
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'parcel' } })
    await screen.findByText('Top 2 of 37 matching sections')
    expect(screen.getByText(/query limit reached/)).toBeTruthy()
    expect(screen.getByText(/Results may be out of date .*: Cannot decode source document/)).toBeTruthy()
    expect(screen.queryByText(/2 results/)).toBeNull()
  })
  it('ignores a superseded request even when transport ignores abort', async () => {
    let oldResolve!: (response: Response) => void
    mockFetch(url => url.includes('q=old') ? new Promise(resolve => { oldResolve = resolve }) : Promise.resolve(new Response(JSON.stringify({ ...response, results: [two] }))))
    render(<DocumentHybridSearch token="test" map="m" locale="en" canSync={false} onNavigate={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Search document/ }))
    const input = screen.getByRole('combobox')
    fireEvent.change(input, { target: { value: 'old' } })
    await waitFor(() => expect(oldResolve).toBeTypeOf('function'))
    fireEvent.change(input, { target: { value: 'new' } })
    await screen.findByText('Shipping')
    await act(async () => oldResolve(new Response(JSON.stringify({ ...response, results: [one] }))))
    expect(screen.queryByText('Delivery')).toBeNull()
    expect(screen.getByText('Shipping')).toBeTruthy()
    expect(screen.queryByText('Sync document')).toBeNull()
  })
})
