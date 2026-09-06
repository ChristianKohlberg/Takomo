import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { insertPlanSection } from './plan-insert'
import { readPlanTree } from './mindmap-crdt'
import { flattenSections, planSections } from './plan-sections'

const outline = (doc: Y.Doc) => flattenSections(planSections(readPlanTree(doc))).map(({ title, number, depth }) => ({ title, number, depth }))

describe('inline document sections', () => {
  it('creates H1/H2/H3 as the shared map hierarchy and syncs to another view', () => {
    const doc = new Y.Doc()
    const h1 = insertPlanSection(doc, null, 1, 'Billing', 'Ada')!
    const h2 = insertPlanSection(doc, h1, 2, 'Invoices', 'Ada')!
    insertPlanSection(doc, h2, 3, 'Due dates', 'Ada')
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    expect(outline(peer)).toEqual([
      { title: 'Billing', number: '1', depth: 0 },
      { title: 'Invoices', number: '1.1', depth: 1 },
      { title: 'Due dates', number: '1.1.1', depth: 2 },
    ])
    doc.destroy(); peer.destroy()
  })

  it('inserts before existing children and between siblings without losing their prose or hierarchy', () => {
    const doc = new Y.Doc()
    const h1 = insertPlanSection(doc, null, 1, 'Billing', 'Ada')!
    const child = insertPlanSection(doc, h1, 2, 'Existing', 'Ada')!
    insertPlanSection(doc, child, 1, 'Reports', 'Ada')
    insertPlanSection(doc, h1, 2, 'First child', 'Ada')
    insertPlanSection(doc, child, 1, 'Payments', 'Ada')
    insertPlanSection(doc, null, 1, 'Overview', 'Ada')
    expect(outline(doc).map(row => [row.title, row.number])).toEqual([
      ['Overview', '1'], ['Billing', '2'], ['First child', '2.1'],
      ['Existing', '2.2'], ['Payments', '3'], ['Reports', '4'],
    ])
    doc.destroy()
  })

  it('refuses orphan headings or a boundary removed by another editor without writing', () => {
    const doc = new Y.Doc()
    expect(insertPlanSection(doc, null, 2, 'Orphan', 'Ada')).toBeNull()
    expect(insertPlanSection(doc, 'deleted', 1, 'Missing', 'Ada')).toBeNull()
    expect(readPlanTree(doc)).toEqual([])
    doc.destroy()
  })
})
