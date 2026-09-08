import * as Y from 'yjs'
import { describe, expect, it, vi } from 'vitest'
import { createNode, deleteSubtree, place, readNodes, setTitle } from './mindmap-crdt'
import { ensureCustomLayout, layoutForMode, readCustomRoot } from './mindmap-custom-layout'
import { layout, radialLayout, type Layout } from './mindmap-layout'

function fixture() {
  const doc = new Y.Doc()
  const a = createNode(doc, { parent: null, title: 'Planning', by: 'test' })!
  const b = createNode(doc, { parent: null, title: 'Delivery', by: 'test' })!
  const child = createNode(doc, { parent: a, title: 'Scope', by: 'test' })!
  return { doc, a, b, child }
}
const positions = (value: Layout) => Object.fromEntries(value.nodes.map(p => [p.node.id, { x: p.x, y: p.y }]))
const custom = (doc: Y.Doc) => layoutForMode(readNodes(doc), 'custom', readCustomRoot(doc))

describe('the persistent custom arrangement', () => {
  it.each(['radial', 'tidy'] as const)('freezes the existing %s arrangement, including manually placed nodes and root', mode => {
    const { doc, b } = fixture()
    place(doc, b, { x: 1600, y: 700 })
    const before = (mode === 'radial' ? radialLayout : layout)(readNodes(doc))
    ensureCustomLayout(doc, mode)
    expect(positions(custom(doc))).toEqual(positions(before))
    expect(readCustomRoot(doc)).toEqual(before.root)
    expect(readNodes(doc).every(n => n.at !== null)).toBe(true)
  })

  it('survives a Yjs reload and repeated view switches without changing content or saved coordinates', () => {
    const { doc, a } = fixture()
    ensureCustomLayout(doc, 'radial')
    place(doc, a, { x: 2400, y: -900 })
    const saved = positions(custom(doc))
    const bytes = Y.encodeStateAsUpdate(doc)
    const reloaded = new Y.Doc()
    Y.applyUpdate(reloaded, bytes)
    const update = vi.fn()
    reloaded.on('update', update)
    const nodes = readNodes(reloaded)
    const before = structuredClone(nodes)
    for (let i = 0; i < 4; i++) {
      for (const mode of ['radial', 'tidy'] as const) {
        const automatic = layoutForMode(nodes, mode)
        const expected = (mode === 'radial' ? radialLayout : layout)(nodes.map(n => ({ ...n, at: null })))
        expect(positions(automatic)).toEqual(positions(expected))
        expect(automatic.root).toEqual(expected.root)
      }
      expect(positions(custom(reloaded))).toEqual(saved)
    }
    expect(nodes).toEqual(before)
    expect(update).not.toHaveBeenCalled()
    expect(Y.encodeStateAsUpdate(reloaded)).toEqual(bytes)
    expect(readCustomRoot(reloaded)).toEqual(readCustomRoot(doc))
  })

  it('places a new child near its moved parent while preserving all existing positions and the root', () => {
    const { doc, a } = fixture()
    ensureCustomLayout(doc, 'radial')
    place(doc, a, { x: 9000, y: 8000 })
    const saved = positions(custom(doc))
    const root = readCustomRoot(doc)
    const child = createNode(doc, { parent: a, title: 'New question', by: 'test' })!
    ensureCustomLayout(doc, 'tidy')
    const after = positions(custom(doc))
    for (const [id, point] of Object.entries(saved)) expect(after[id]).toEqual(point)
    expect(Math.abs(after[child]!.x - 9000)).toBeLessThan(1000)
    expect(Math.abs(after[child]!.y - 8000)).toBeLessThan(1000)
    expect(after[child]).not.toEqual(after[a])
    expect(readCustomRoot(doc)).toEqual(root)
  })

  it('is idempotent and does not restore deleted nodes or stale titles and parents', () => {
    const { doc, a, b, child } = fixture()
    ensureCustomLayout(doc, 'tidy')
    deleteSubtree(doc, b)
    setTitle(doc, a, 'Revised plan')
    const bytes = Y.encodeStateAsUpdate(doc)
    const update = vi.fn()
    doc.on('update', update)
    ensureCustomLayout(doc, 'radial')
    ensureCustomLayout(doc, 'tidy')
    expect(Y.encodeStateAsUpdate(doc)).toEqual(bytes)
    expect(update).not.toHaveBeenCalled()
    expect(readNodes(doc).find(n => n.id === b)).toBeUndefined()
    expect(readNodes(doc).find(n => n.id === a)?.title).toBe('Revised plan')
    expect(readNodes(doc).find(n => n.id === child)?.parent).toBe(a)
  })

  it('keeps different maps independent', () => {
    const first = fixture()
    const second = fixture()
    ensureCustomLayout(first.doc, 'radial')
    ensureCustomLayout(second.doc, 'tidy')
    const before = Y.encodeStateAsUpdate(second.doc)
    place(first.doc, first.a, { x: -9000, y: -5000 })
    ensureCustomLayout(first.doc, 'tidy')
    expect(Y.encodeStateAsUpdate(second.doc)).toEqual(before)
    expect(readCustomRoot(first.doc)).not.toEqual(readCustomRoot(second.doc))
  })
})

it('merges concurrent custom moves while an automatic-view replica publishes no layout updates', () => {
  const { doc: alice, a, b } = fixture()
  ensureCustomLayout(alice, 'radial')
  const bob = new Y.Doc()
  Y.applyUpdate(bob, Y.encodeStateAsUpdate(alice))
  const bobUpdates = vi.fn()
  bob.on('update', bobUpdates)
  layoutForMode(readNodes(bob), 'tidy')
  layoutForMode(readNodes(bob), 'radial')
  expect(bobUpdates).not.toHaveBeenCalled()

  // Two different nodes are moved offline, then the updates cross on reconnect.
  place(alice, a, { x: 1500, y: 800 })
  place(bob, b, { x: -1200, y: -600 })
  const aliceUpdate = Y.encodeStateAsUpdate(alice)
  const bobUpdate = Y.encodeStateAsUpdate(bob)
  Y.applyUpdate(alice, bobUpdate)
  Y.applyUpdate(bob, aliceUpdate)
  expect(positions(custom(alice))).toEqual(positions(custom(bob)))
  expect(positions(custom(alice))[a]).toEqual({ x: 1500, y: 800 })
  expect(positions(custom(alice))[b]).toEqual({ x: -1200, y: -600 })
  expect(readCustomRoot(alice)).toEqual(readCustomRoot(bob))

  bobUpdates.mockClear()
  layoutForMode(readNodes(bob), 'tidy')
  layoutForMode(readNodes(bob), 'custom', readCustomRoot(bob))
  ensureCustomLayout(bob, 'tidy')
  expect(bobUpdates).not.toHaveBeenCalled()
})
