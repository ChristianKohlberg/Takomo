import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import type { WebsocketProvider } from 'y-websocket'
import { afterEach, expect, it, vi } from 'vitest'
import { trackSave, type SaveState } from './save-status'
vi.mock('./durable-replica', () => ({ persistReplica: (_doc: unknown, _key: string, changed: (state: string) => void) => {
  changed('saved'); return { ready: Promise.resolve(), destroy: vi.fn() }
} }))
afterEach(() => vi.useRealTimers())
it('requires an acknowledgement of the latest edit, including deletions, before reporting saved', async () => {
  vi.useFakeTimers()
  const doc = new Y.Doc()
  const listeners: Record<string, (value?: boolean) => void> = {}
  const send = vi.fn()
  const provider = { wsconnected: true, synced: true, ws: { send }, messageHandlers: [],
    on: (event: string, fn: (value?: boolean) => void) => { listeners[event] = fn }, off: vi.fn() } as unknown as WebsocketProvider
  let state: SaveState = 'connecting'
  const tracker = trackSave(doc, provider, { object: 'mm-1', actor: 'alice', durability_ack: true }, value => { state = value })
  await tracker.ready
  expect(state).toBe('syncing')
  doc.getText('body').insert(0, 'draft')
  await vi.advanceTimersByTimeAsync(600)
  const sent = decoding.createDecoder(send.mock.calls[0]![0])
  expect(decoding.readVarUint(sent)).toBe(4)
  const revision = decoding.readVarUint(sent)
  const ack = (seq: number, saved = true) => {
    const encoder = encoding.createEncoder(); encoding.writeVarUint(encoder, seq); encoding.writeVarUint(encoder, saved ? 1 : 0)
    provider.messageHandlers[4]!(encoding.createEncoder(), decoding.createDecoder(encoding.toUint8Array(encoder)), provider, true, 4)
  }
  doc.getText('body').delete(0, 1)
  ack(revision)
  expect(state).toBe('syncing')
  ack(revision + 1, false)
  expect(state).toBe('server-error')
  ack(revision + 1)
  expect(state).toBe('saved')
  provider.wsconnected = false; listeners.status!()
  expect(state).toBe('offline')
  provider.wsconnected = true; listeners.status!()
  expect(state).toBe('syncing')
  tracker.destroy(); doc.destroy()
})
