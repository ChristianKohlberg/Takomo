import { expect, it } from 'vitest'
import * as Y from 'yjs'
import { createNode, readNodes, place } from './mindmap-crdt'
import { visibleNodes } from './mindmap-doc'
import { focusBranch, focusedLayout } from './mindmap-focus'
it('projects only the branch, respects folds, and does not change stored parents or positions', () => {
  const doc = new Y.Doc()
  const parent = createNode(doc, { parent: null, title: 'Product', by: 'test' })!
  const root = createNode(doc, { parent, title: 'Payments', by: 'test' })!
  const child = createNode(doc, { parent: root, title: 'Retry', by: 'test' })!
  createNode(doc, { parent, title: 'Unrelated', by: 'test' })
  place(doc, root, { x: 900, y: 1200 })
  const nodes = readNodes(doc), branch = focusBranch(nodes, root)
  expect(branch.map(n => n.id)).toEqual([root, child])
  expect(branch[0]!.parent).toBeNull()
  expect(nodes.find(n => n.id === root)?.parent).toBe(parent)
  expect(visibleNodes(branch, new Set([root])).map(n => n.id)).toEqual([root])
  const placed = focusedLayout(branch, 'custom', { x: -10000, y: -10000 }, root)
  expect(placed.nodes.find(n => n.node.id === root)).toMatchObject({ x: 900, y: 1200 })
  expect(placed.bounds.minX).toBeGreaterThan(-10000)
  expect(readNodes(doc)).toEqual(nodes)
  expect(focusBranch(nodes, null)).toBe(nodes)
})
