import 'fake-indexeddb/auto'
import * as Y from 'yjs'
import { expect, it, vi } from 'vitest'
import { persistReplica, type LocalSave } from './durable-replica'

it('restores an offline edit and deletion after closing the replica', async () => {
  const key = crypto.randomUUID()
  const doc = new Y.Doc()
  let state: LocalSave = 'loading'
  const persistence = persistReplica(doc, key, value => { state = value })
  await persistence.ready
  doc.getText('body').insert(0, 'offline draft')
  doc.getText('body').delete(0, 8)
  await vi.waitFor(() => expect(state).toBe('saved'))
  persistence.destroy(); doc.destroy()
  const restored = new Y.Doc()
  const next = persistReplica(restored, key, () => {})
  await next.ready
  expect(restored.getText('body').toString()).toBe('draft')
  next.destroy(); restored.destroy()
})

it('retains independent offline writes from two tabs when compacting the log', async () => {
  const key = crypto.randomUUID()
  const a = new Y.Doc(), b = new Y.Doc()
  let sa: LocalSave = 'loading', sb: LocalSave = 'loading'
  const pa = persistReplica(a, key, value => { sa = value })
  const pb = persistReplica(b, key, value => { sb = value })
  await Promise.all([pa.ready, pb.ready])
  a.getText('a').insert(0, 'first tab')
  for (let i = 0; i < 140; i++) b.getText('b').insert(i, 'b')
  await vi.waitFor(() => { expect(sa).toBe('saved'); expect(sb).toBe('saved') })
  pa.destroy(); pb.destroy(); a.destroy(); b.destroy()
  const restored = new Y.Doc()
  const p = persistReplica(restored, key, () => {})
  await p.ready
  expect(restored.getText('a').toString()).toBe('first tab')
  expect(restored.getText('b').length).toBe(140)
  p.destroy(); restored.destroy()
})

it('finishes a queued write even when navigation destroys the editor immediately', async () => {
  const key = crypto.randomUUID()
  const doc = new Y.Doc()
  const persistence = persistReplica(doc, key, () => {})
  await persistence.ready
  doc.getText('body').insert(0, 'last keystroke')
  persistence.destroy(); doc.destroy()
  const restored = new Y.Doc()
  const next = persistReplica(restored, key, () => {})
  await next.ready
  expect(restored.getText('body').toString()).toBe('last keystroke')
  next.destroy(); restored.destroy()
})
