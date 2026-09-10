// @vitest-environment jsdom
import { StrictMode } from 'react'
import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useLiveRefresh } from './useLiveRefresh'
import type { ProjectUpdate } from './useProjectUpdates'
const bus = vi.hoisted(() => ({ connected: true, update: undefined as undefined | ((event?: ProjectUpdate) => Promise<unknown>) }))
vi.mock('./useProjectUpdates', async importOriginal => ({
  ...await importOriginal<typeof import('./useProjectUpdates')>(),
  useProjectUpdates: (_token: string, _project: string, update: typeof bus.update) => { bus.update = update; return bus.connected },
}))
const options = { token: 'token', project: 'one', scope: 'resource', topics: ['inbox'] as ('inbox')[] }
const advance = async (ms: number) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms) }) }
const event = async () => { await act(async () => { await bus.update?.({ type: 'refresh', topics: ['inbox'] }) }) }
const visibility = (state: string) => act(() => { Object.defineProperty(document, 'visibilityState', { configurable: true, value: state }); document.dispatchEvent(new Event('visibilitychange')) })
beforeEach(() => { vi.useFakeTimers(); bus.connected = true; visibility('visible') })
afterEach(() => { cleanup(); vi.useRealTimers() })
it('stays idle when connected, falls back disconnected and defers hidden events', async () => {
  const load = vi.fn().mockResolvedValue(undefined)
  const { rerender } = renderHook(() => useLiveRefresh({ ...options, load }))
  await advance(60000)
  expect(load).toHaveBeenCalledTimes(1)
  bus.connected = false; rerender()
  await advance(30000)
  expect(load).toHaveBeenCalledTimes(2)
  visibility('hidden'); await event(); await advance(60000)
  expect(load).toHaveBeenCalledTimes(2)
  bus.connected = true; rerender(); visibility('visible'); await advance(0)
  expect(load).toHaveBeenCalledTimes(3)
  await advance(60000)
  expect(load).toHaveBeenCalledTimes(3)
})
it('coalesces events during a request and enforces the active cadence', async () => {
  let finish!: () => void
  const load = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve })).mockResolvedValue(undefined)
  renderHook(() => useLiveRefresh({ ...options, load, activeMs: 1000 }))
  await event(); await event(); await advance(400)
  await act(async () => finish())
  await advance(599); expect(load).toHaveBeenCalledTimes(1)
  await advance(1); expect(load).toHaveBeenCalledTimes(2)
  await event(); await event(); await advance(999)
  expect(load).toHaveBeenCalledTimes(2)
  await advance(1); expect(load).toHaveBeenCalledTimes(3)
})
it('aborts a read when undo pauses it and preserves the pending invalidation', async () => {
  const signals: AbortSignal[] = []
  const load = vi.fn(async (signal: AbortSignal) => { signals.push(signal); if (signals.length === 1) await new Promise(() => {}) })
  const { rerender } = renderHook(({ paused }) => useLiveRefresh({ ...options, load, paused }), { initialProps: { paused: false } })
  rerender({ paused: true }); await event(); await advance(40000)
  expect(signals[0]!.aborted).toBe(true)
  expect(load).toHaveBeenCalledTimes(1)
  rerender({ paused: false }); await advance(0)
  expect(load).toHaveBeenCalledTimes(2)
})
it('retries transient errors but stops auth failures and preserves manual mode', async () => {
  const onError = vi.fn()
  const load = vi.fn().mockRejectedValueOnce(new Error('offline')).mockRejectedValueOnce({ status: 401, auth: true })
  renderHook(() => useLiveRefresh({ ...options, load, onError }))
  await advance(30000); expect(load).toHaveBeenCalledTimes(2)
  await event(); await advance(60000); expect(load).toHaveBeenCalledTimes(2)
  expect(onError).toHaveBeenCalledTimes(2)
})
it('ignores late work across scopes and survives StrictMode and manual refresh', async () => {
  const load = vi.fn().mockResolvedValue(undefined)
  const { result, rerender } = renderHook(({ scope }) => useLiveRefresh({ ...options, scope, load, automatic: false }), { initialProps: { scope: 'one' }, wrapper: StrictMode })
  await advance(0)
  const initial = load.mock.calls.length
  await event(); await advance(60000)
  expect(load).toHaveBeenCalledTimes(initial)
  act(() => result.current.refresh()); await advance(0)
  expect(load).toHaveBeenCalledTimes(initial + 1)
  rerender({ scope: 'two' }); await advance(0)
  expect(load).toHaveBeenCalledTimes(initial + 2)
})
it('does not let a late aborted completion erase the resumed request retry', async () => {
  let finishOld!: () => void
  const load = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finishOld = resolve }))
    .mockRejectedValueOnce(new Error('new request failed')).mockResolvedValue(undefined)
  const { rerender } = renderHook(({ paused }) => useLiveRefresh({ ...options, load, paused }), { initialProps: { paused: false } })
  rerender({ paused: true }); await advance(1000)
  rerender({ paused: false }); await advance(0)
  expect(load).toHaveBeenCalledTimes(2)
  await advance(10000); await act(async () => finishOld())
  await advance(20000)
  expect(load).toHaveBeenCalledTimes(3)
})
it('backs off failed active reads and retains revoked credentials across resource changes', async () => {
  const load = vi.fn().mockRejectedValueOnce(new Error('offline')).mockRejectedValueOnce({ status: 401 }).mockResolvedValue(undefined)
  const { rerender } = renderHook(({ token, scope }) => useLiveRefresh({ ...options, token, scope, load, activeMs: 1000 }), { initialProps: { token: 'old', scope: 'one' } })
  await advance(1000); expect(load).toHaveBeenCalledTimes(1)
  await advance(29000); expect(load).toHaveBeenCalledTimes(2)
  rerender({ token: 'old', scope: 'two' }); await advance(60000)
  expect(load).toHaveBeenCalledTimes(2)
  rerender({ token: 'new', scope: 'two' }); await advance(0)
  expect(load).toHaveBeenCalledTimes(3)
})
it('refreshes once when automatic updates resume after ignored notifications', async () => {
  const load = vi.fn().mockResolvedValue(undefined)
  const { rerender } = renderHook(({ automatic }) => useLiveRefresh({ ...options, load, automatic }), { initialProps: { automatic: false } })
  await advance(5000); await event()
  expect(load).toHaveBeenCalledTimes(1)
  rerender({ automatic: true }); await advance(0)
  expect(load).toHaveBeenCalledTimes(2)
  await advance(60000)
  expect(load).toHaveBeenCalledTimes(2)
})
