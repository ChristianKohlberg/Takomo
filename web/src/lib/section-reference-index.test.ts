import { between } from './fracdex'
import { expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { createNode, nodesMap, setTitle } from './mindmap-crdt'
import { referenceLabel, sectionReferenceIndex, searchReferenceSections } from './section-reference-index'
it('shares an index, ignores prose metadata and follows remote hierarchy/title changes', () => {
  const doc = new Y.Doc()
  const a = createNode(doc, { title: 'Billing', parent: null, by: 'test' })!
  const b = createNode(doc, { title: 'Receipts', parent: null, by: 'test' })!
  const index = sectionReferenceIndex(doc); const snapshot = index.getSnapshot(); const notify = vi.fn(); const unsubscribe = index.subscribe(notify)
  expect(sectionReferenceIndex(doc)).toBe(index)
  nodesMap(doc).get(a)!.set('reviewed', true)
  expect(index.getSnapshot()).toBe(snapshot); expect(notify).not.toHaveBeenCalled()
  const remote = new Y.Doc(); Y.applyUpdate(remote, Y.encodeStateAsUpdate(doc))
  remote.transact(() => { nodesMap(remote).get(b)!.set('parent', a); setTitle(remote, b, 'Invoices') })
  Y.applyUpdate(doc, Y.encodeStateAsUpdate(remote))
  expect(referenceLabel(doc, b, 'Untitled')).toBe('1.1 Invoices')
  expect(searchReferenceSections(index.getSnapshot(), '1.1 inv')[0]?.key).toBe(b)
  expect(notify).toHaveBeenCalledTimes(1)
  const manager = new Y.UndoManager(nodesMap(doc)); nodesMap(doc).delete(b)
  expect(referenceLabel(doc, b, 'Untitled')).toBeNull()
  manager.undo(); expect(referenceLabel(doc, b, 'Untitled')).toBe('1.1 Invoices')
  manager.destroy(); unsubscribe(); remote.destroy(); doc.destroy()
})
it('renumbers an untouched target after sibling insertions and reorders without retargeting a deleted identity', () => {
  const doc = new Y.Doc()
  const target = createNode(doc, { title: 'Same title', parent: null, by: 'test' })!
  const index = sectionReferenceIndex(doc)
  expect(referenceLabel(doc, target, '')).toBe('1 Same title')
  const sibling = createNode(doc, { title: 'Same title', parent: null, by: 'test' })!
  nodesMap(doc).get(sibling)!.set('order', between(null, nodesMap(doc).get(target)!.get('order') as string))
  expect(referenceLabel(doc, target, '')).toBe('2 Same title')
  nodesMap(doc).get(sibling)!.set('order', between(nodesMap(doc).get(target)!.get('order') as string, null))
  expect(referenceLabel(doc, target, '')).toBe('1 Same title')
  nodesMap(doc).delete(target)
  expect(referenceLabel(doc, target, '')).toBeNull()
  expect(index.getSnapshot()[0]?.key).toBe(sibling)
  doc.destroy()
})
