import { describe, expect, it } from 'vitest'

import { buildTree, type Doc } from './documents'

function doc(id: string, title: string, path: string): Doc {
  return {
    id,
    project: 'tp',
    title,
    path,
    status: 'draft',
    initiative: null,
    metadata: null,
    version: 1,
    created_by: 'test',
    created_at: '2026-08-23T00:00:00.000Z',
    updated_at: '2026-08-23T00:00:00.000Z',
    archived_at: null,
    bytes: 0,
    updates: 0,
  }
}

describe('buildTree', () => {
  it('puts pathless documents at the top level', () => {
    const tree = buildTree([doc('a', 'Alpha', ''), doc('b', 'Beta', '')])
    expect(tree.docs.map((d) => d.title)).toEqual(['Alpha', 'Beta'])
    expect(tree.children).toEqual([])
  })

  it('creates a folder because a document names it, and only then', () => {
    // The rule `initiative-tree.ts` follows: no folder table, so no folder can
    // exist that nothing is filed in — and the last document to leave takes the
    // folder with it.
    const tree = buildTree([doc('a', 'Rates', 'product/billing')])
    expect(tree.docs).toEqual([])
    const product = tree.children[0]!
    expect(product.name).toBe('product')
    expect(product.docs).toEqual([])
    const billing = product.children[0]!
    expect(billing.name).toBe('billing')
    expect(billing.path).toBe('product/billing')
    expect(billing.docs.map((d) => d.title)).toEqual(['Rates'])
  })

  it('shares intermediate folders between siblings rather than duplicating them', () => {
    const tree = buildTree([
      doc('a', 'Rates', 'product/billing'),
      doc('b', 'Invoices', 'product/billing'),
      doc('c', 'Roadmap', 'product'),
    ])
    expect(tree.children).toHaveLength(1)
    const product = tree.children[0]!
    expect(product.docs.map((d) => d.title)).toEqual(['Roadmap'])
    expect(product.children).toHaveLength(1)
    expect(product.children[0]!.docs.map((d) => d.title)).toEqual(['Invoices', 'Rates'])
  })

  it('sorts folders and documents by name so the tree does not reshuffle between reads', () => {
    const tree = buildTree([
      doc('a', 'Zebra', 'zulu'),
      doc('b', 'Alpha', 'alpha'),
      doc('c', 'Middle', ''),
    ])
    expect(tree.children.map((f) => f.name)).toEqual(['alpha', 'zulu'])
    expect(tree.docs.map((d) => d.title)).toEqual(['Middle'])
  })
})
