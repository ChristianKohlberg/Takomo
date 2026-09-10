// @vitest-environment jsdom
import { StrictMode } from 'react'
import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { api } from '@/lib/api'
import { affectsProjectTopic, parseProjectUpdate, useProjectUpdates } from './useProjectUpdates'
vi.mock('@/lib/api', () => ({ api: vi.fn() }))
vi.mock('@/lib/collab-session', () => ({ syncBase: () => 'ws://localhost' }))
class Socket {
  static instances: Socket[] = []
  onmessage?: (event: { data: string }) => void
  onclose?: () => void
  onerror?: () => void
  constructor() { Socket.instances.push(this) }
  close = vi.fn()
  message(value: unknown) { this.onmessage?.({ data: JSON.stringify(value) }) }
}
beforeEach(() => { Socket.instances = []; vi.spyOn(Math, 'random').mockReturnValue(0); vi.mocked(api).mockClear(); vi.stubGlobal('WebSocket', Socket); vi.mocked(api).mockResolvedValue({ room: 'one', token: 'ticket' }) })
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); vi.restoreAllMocks() })
it('filters topics and conservatively treats legacy or unknown topics as full recovery', () => {
  expect(affectsProjectTopic(parseProjectUpdate('{"type":"refresh","topics":["trace"]}')!, 'inbox')).toBe(false)
  expect(affectsProjectTopic(parseProjectUpdate('{"type":"refresh"}')!, 'inbox')).toBe(true)
  expect(affectsProjectTopic(parseProjectUpdate('{"type":"refresh","topics":["future"]}')!, 'inbox')).toBe(true)
  expect(parseProjectUpdate('{"type":"ping"}')).toBeNull()
})
it('reports readiness, coalesces pending topic updates and disconnects without extra refreshes', async () => {
  let finish!: () => void
  const refresh = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve })).mockResolvedValue(undefined)
  const { result } = renderHook(() => useProjectUpdates('token', 'one', refresh))
  await waitFor(() => expect(Socket.instances).toHaveLength(1))
  expect(result.current).toBeFalsy()
  act(() => Socket.instances[0]!.message({ type: 'refresh' }))
  expect(result.current).toBe(true)
  act(() => { Socket.instances[0]!.message({ type: 'refresh', topics: ['inbox'] }); Socket.instances[0]!.message({ type: 'refresh', topics: ['trace'] }) })
  await act(async () => finish())
  expect(refresh).toHaveBeenCalledTimes(2)
  expect(refresh.mock.calls[1]![0].topics).toEqual(['inbox', 'trace'])
  act(() => Socket.instances[0]!.onclose?.())
  expect(result.current).toBe(false)
  expect(refresh).toHaveBeenCalledTimes(2)
})

it('reconnects after two seconds and uses the initial generic event for one recovery refresh', async () => {
  vi.useFakeTimers()
  const refresh = vi.fn().mockResolvedValue(undefined)
  const { result } = renderHook(() => useProjectUpdates('token', 'one', refresh))
  await act(async () => { await Promise.resolve() })
  await act(async () => Socket.instances[0]!.message({ type: 'refresh', topics: ['inbox'] }))
  expect(result.current).toBe(true)
  act(() => Socket.instances[0]!.onclose?.())
  expect(result.current).toBe(false)
  await act(async () => { await vi.advanceTimersByTimeAsync(2000) })
  expect(Socket.instances).toHaveLength(2)
  expect(refresh).toHaveBeenCalledTimes(1)
  await act(async () => Socket.instances[1]!.message({ type: 'refresh' }))
  expect(result.current).toBe(true)
  expect(refresh).toHaveBeenCalledTimes(2)
  expect(refresh.mock.calls[1]![0]).toEqual({ type: 'refresh' })
})

it('shares one transport, resyncs late consumers and releases only the last subscriber', async () => {
  const first = renderHook(() => useProjectUpdates('token', 'one', async () => {}))
  await waitFor(() => expect(Socket.instances).toHaveLength(1))
  await act(async () => Socket.instances[0]!.message({ type: 'refresh' }))
  const refresh = vi.fn().mockResolvedValue(undefined)
  const second = renderHook(() => useProjectUpdates('token', 'one', refresh))
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1))
  expect(api).toHaveBeenCalledTimes(1)
  first.unmount()
  expect(Socket.instances[0]!.close).not.toHaveBeenCalled()
  second.unmount()
  expect(Socket.instances[0]!.close).toHaveBeenCalledTimes(1)
})
it('isolates credentials and projects and stops terminal authentication retries', async () => {
  vi.useFakeTimers()
  const error = { status: 401, auth: true }
  vi.mocked(api).mockRejectedValueOnce(error)
  const onError = vi.fn()
  renderHook(() => useProjectUpdates('bad', 'one', async () => {}, onError))
  await act(async () => { await vi.advanceTimersByTimeAsync(0) })
  expect(onError).toHaveBeenCalledWith(error)
  await act(async () => { await vi.advanceTimersByTimeAsync(60000) })
  expect(api).toHaveBeenCalledTimes(1)
  const lateError = vi.fn()
  renderHook(() => useProjectUpdates('bad', 'one', async () => {}, lateError))
  await act(async () => { await vi.advanceTimersByTimeAsync(0) })
  expect(lateError).toHaveBeenCalledWith(error)
  renderHook(() => useProjectUpdates('good', 'one', async () => {}))
  renderHook(() => useProjectUpdates('good', 'two', async () => {}))
  await act(async () => { await vi.advanceTimersByTimeAsync(0) })
  expect(Socket.instances).toHaveLength(2)
})

it('does not leak pending connections during StrictMode cleanup', async () => {
  let finish!: (value: unknown) => void
  vi.mocked(api).mockImplementation(() => new Promise(resolve => { finish = resolve }))
  const view = renderHook(() => useProjectUpdates('token', 'one', async () => {}), { wrapper: StrictMode })
  await act(async () => { await Promise.resolve() })
  view.unmount()
  await act(async () => finish({ room: 'one', token: 'ticket' }))
  expect(Socket.instances).toHaveLength(0)
})
