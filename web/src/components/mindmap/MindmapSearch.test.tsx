import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { createNode, readNodes } from '@/lib/mindmap-crdt'
import { visibleNodes } from '@/lib/mindmap-doc'
import { searchDocument, type SearchResponse } from '@/lib/hybrid-search'
import { MindmapSearch, searchFolds, useMindmapSearch } from './MindmapSearch'
vi.mock('@/lib/hybrid-search', () => ({ searchDocument: vi.fn() }))
afterEach(() => vi.resetAllMocks())
const response = (id: string): SearchResponse => ({ results: [{ node_id: id, title: 'Delivery', heading_path: [], excerpt: 'parcel', passage: 'parcel', highlights: [], match_kind: 'semantic' }], limit: 40, candidates: 1, truncated: false, mode: 'hybrid', semantic_status: 'ready', projection: 'current', projection_error: null })
it('uses content results and restores folded branches on clear', async () => {
  const doc = new Y.Doc()
  const root = createNode(doc, { parent: null, title: 'Operations', by: 'test' })!
  const child = createNode(doc, { parent: root, title: 'Delivery', by: 'test' })!
  const nodes = readNodes(doc), folds = new Set([root]), answer = response(child)
  answer.results.push(answer.results[0]!, { ...answer.results[0]!, node_id: 'deleted' })
  vi.mocked(searchDocument).mockResolvedValue(answer)
  const { result } = renderHook(() => useMindmapSearch('human', 'map', nodes))
  act(() => result.current.setQuery('parcel'))
  await waitFor(() => expect(result.current.matches.has(child)).toBe(true))
  expect(searchDocument).toHaveBeenCalledWith('human', 'map', 'parcel', expect.any(AbortSignal))
  expect(result.current.matches.size).toBe(1)
  expect(visibleNodes(nodes, searchFolds(nodes, folds, result.current.matches))).toHaveLength(2)
  expect(folds.has(root)).toBe(true)
  act(() => result.current.setQuery(''))
  expect(result.current.matches.size).toBe(0)
  expect(visibleNodes(nodes, searchFolds(nodes, folds, result.current.matches))).toHaveLength(1)
})
it('aborts requests and ignores late results after clearing', async () => {
  let finish!: (value: SearchResponse) => void
  vi.mocked(searchDocument).mockImplementation(() => new Promise(resolve => { finish = resolve }))
  const { result } = renderHook(() => useMindmapSearch('human', 'map', []))
  act(() => result.current.setQuery('old'))
  await waitFor(() => expect(searchDocument).toHaveBeenCalledTimes(1))
  const signal = vi.mocked(searchDocument).mock.calls[0]![3]!
  act(() => result.current.setQuery(''))
  expect(signal.aborted).toBe(true)
  await act(async () => finish(response('one')))
  expect(result.current.response).toBeUndefined()
  expect(result.current.busy).toBe(false)
})
it('shows errors and clears them for a new query', async () => {
  vi.mocked(searchDocument).mockRejectedValue(new Error('Search unavailable'))
  const { result } = renderHook(() => useMindmapSearch('human', 'map', []))
  act(() => result.current.setQuery('parcel'))
  await waitFor(() => expect(result.current.error).toBe('Search unavailable'))
  act(() => result.current.setQuery(''))
  expect(result.current.error).toBeUndefined()
})
it('focuses inline with Ctrl S and steps through matches without opening a dialog', () => {
  const navigate = vi.fn(), setQuery = vi.fn()
  render(<MindmapSearch query="parcel" setQuery={setQuery} response={response('one')} matches={new Set(['one', 'two'])} error={undefined} busy={false} locale="en" onNavigate={navigate} />)
  expect(navigate).not.toHaveBeenCalled()
  expect(screen.queryByRole('dialog')).toBeNull()
  fireEvent.keyDown(window, { ctrlKey: true, key: 's' })
  expect(document.activeElement).toBe(screen.getByRole('searchbox'))
  fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Enter' })
  expect(navigate).toHaveBeenLastCalledWith('one')
  fireEvent.click(screen.getByRole('button', { name: 'Previous match' }))
  expect(navigate).toHaveBeenLastCalledWith('two')
  fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' })
  expect(setQuery).toHaveBeenCalledWith('')
})

it('keeps outside matches separate and only exits focus on request', () => {
  const onShowAll = vi.fn(), onNavigate = vi.fn()
  render(<MindmapSearch query="parcel" setQuery={vi.fn()} response={response('one')} matches={new Set(['one'])} error={undefined} busy={false} locale="en" onNavigate={onNavigate} outsideMatches={3} onShowAll={onShowAll} />)
  fireEvent.click(screen.getByRole('button', { name: 'Next match' }))
  expect(onNavigate).toHaveBeenCalledWith('one')
  expect(onShowAll).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: '3 matches outside this branch · Show full map' }))
  expect(onShowAll).toHaveBeenCalledOnce()
})
