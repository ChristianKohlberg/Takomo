// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { getTrace } from '@/lib/mindmaps'
import { useSectionTrace } from './useSectionTrace'
vi.mock('@/lib/mindmaps', () => ({ getTrace: vi.fn() }))
beforeEach(() => { vi.mocked(getTrace).mockReset() })
afterEach(cleanup)
const page = (id: string) => ({ items: [{ id, node: id }], total: 1, limit: 500 }) as Awaited<ReturnType<typeof getTrace>>
it('loads only disclosed sections, reuses clean caches, and marks closed history dirty without fetching', async () => {
  vi.mocked(getTrace).mockImplementation(async (_token, _map, filter) => page(filter!.node!))
  const { result } = renderHook(() => useSectionTrace('token', 'map'))
  expect(getTrace).not.toHaveBeenCalled()
  act(() => { result.current.setOpen('one', true); result.current.setOpen('one', true); result.current.setOpen('two', true) })
  await waitFor(() => expect(result.current.cache.two!.loading).toBe(false))
  expect(getTrace).toHaveBeenCalledTimes(2)
  expect(result.current.cache.one!.entries[0]!.id).toBe('one')
  expect(result.current.cache.two!.entries[0]!.id).toBe('two')
  act(() => { result.current.setOpen('one', false); result.current.setOpen('two', false); result.current.invalidate() })
  expect(getTrace).toHaveBeenCalledTimes(2)
  act(() => result.current.setOpen('one', true))
  await waitFor(() => expect(getTrace).toHaveBeenCalledTimes(3))
  await waitFor(() => expect(result.current.cache.one!.loading).toBe(false))
  act(() => { result.current.setOpen('one', false); result.current.setOpen('one', true) })
  expect(getTrace).toHaveBeenCalledTimes(3)
})
it('retains invalidation during an in-flight request and ignores obsolete scope responses', async () => {
  let finish!: (value: Awaited<ReturnType<typeof getTrace>>) => void
  vi.mocked(getTrace).mockReturnValueOnce(new Promise(resolve => { finish = resolve })).mockResolvedValue(page('new'))
  const { result, rerender } = renderHook(({ token }) => useSectionTrace(token, 'map'), { initialProps: { token: 'first' } })
  act(() => result.current.setOpen('one', true))
  act(() => result.current.invalidate())
  await act(async () => finish(page('old')))
  await waitFor(() => expect(getTrace).toHaveBeenCalledTimes(2))
  expect(result.current.cache.one!.entries[0]!.id).toBe('new')
  act(() => result.current.setOpen('one', false))
  vi.mocked(getTrace).mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
  act(() => { result.current.invalidate(); result.current.setOpen('one', true) })
  rerender({ token: 'second' })
  await act(async () => finish(page('secret-old')))
  expect(result.current.cache).toEqual({})
})
it('shows a failed load and retries explicitly without a request loop', async () => {
  vi.mocked(getTrace).mockRejectedValueOnce(new Error('offline')).mockResolvedValue(page('recovered'))
  const { result } = renderHook(() => useSectionTrace('token', 'map'))
  act(() => result.current.setOpen('one', true))
  await waitFor(() => expect(result.current.cache.one!.error).toBe(true))
  expect(getTrace).toHaveBeenCalledTimes(1)
  await act(() => result.current.retry('one'))
  expect(result.current.cache.one!.error).toBe(false)
  expect(result.current.cache.one!.entries[0]!.id).toBe('recovered')
})
