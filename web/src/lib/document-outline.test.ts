import { describe, expect, it } from 'vitest'

import {
  ancestorKeys,
  buildOutline,
  sectionCount,
  visibleSections,
  type OutlineSection,
} from './document-outline'
import type { Doc } from './documents'

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

/** Every section, depth-first — the shape most assertions here want to read. */
function flat(sections: readonly OutlineSection[]): OutlineSection[] {
  return visibleSections(sections, new Set())
}

const addresses = (sections: readonly OutlineSection[]): string[] =>
  flat(sections).map((s) => `${s.number} ${s.title}`)

describe('buildOutline', () => {
  it('puts pathless documents at the top level, numbered from one', () => {
    const out = buildOutline([doc('a', 'Alpha', ''), doc('b', 'Beta', '')])
    expect(addresses(out)).toEqual(['1 Alpha', '2 Beta'])
    expect(out.every((s) => s.children.length === 0)).toBe(true)
  })

  it('folds a folder and the document that named it into ONE section', () => {
    // This is the case the conversion produces: `POST /v1/mindmaps/{id}/documents`
    // writes `API` at `Payments rebuild` and files its children at
    // `Payments rebuild/API`. Two rows called API would read as a bug.
    const out = buildOutline([
      doc('a', 'API', 'Payments rebuild'),
      doc('b', 'Versioning', 'Payments rebuild/API'),
      doc('c', 'Payments rebuild', ''),
    ])
    expect(addresses(out)).toEqual(['1 Payments rebuild', '1.1 API', '1.1.1 Versioning'])
    const api = out[0]!.children[0]!
    expect(api.doc?.id).toBe('a')
    expect(api.folder).toBe('Payments rebuild/API')
    expect(api.key).toBe('a')
  })

  it('keeps a folder no document names as a plain group', () => {
    // Somebody filed by hand. The group is still addressable and still opens;
    // it just has no prose behind it.
    const out = buildOutline([doc('a', 'Rates', 'product/billing')])
    expect(addresses(out)).toEqual(['1 product', '1.1 billing', '1.1.1 Rates'])
    const product = out[0]!
    expect(product.doc).toBeNull()
    expect(product.key).toBe('product')
    expect(product.children[0]!.doc).toBeNull()
    expect(product.children[0]!.children[0]!.doc?.id).toBe('a')
  })

  it('leaves a document that names no folder as a leaf', () => {
    const out = buildOutline([doc('a', 'API', 'Payments'), doc('b', 'Payments', '')])
    const payments = out[0]!
    expect(payments.doc?.id).toBe('b')
    expect(payments.children).toHaveLength(1)
    expect(payments.children[0]!.folder).toBeNull()
    expect(payments.children[0]!.children).toEqual([])
  })

  it('shares intermediate folders between siblings rather than duplicating them', () => {
    const out = buildOutline([
      doc('a', 'Rates', 'product/billing'),
      doc('b', 'Invoices', 'product/billing'),
      doc('c', 'Roadmap', 'product'),
    ])
    expect(out).toHaveLength(1)
    const product = out[0]!
    expect(product.children.map((s) => s.title)).toEqual(['billing', 'Roadmap'])
    expect(product.children[0]!.children.map((s) => s.title)).toEqual(['Invoices', 'Rates'])
  })

  it('orders by title and does not hoist folders above documents', () => {
    // Once a folder can BE a document there is no second kind to sort
    // separately, and interleaving is what makes the numbering read as one
    // sequence rather than two.
    const out = buildOutline([
      doc('a', 'Zebra', 'zulu'),
      doc('b', 'Alpha', 'alpha'),
      doc('c', 'Middle', ''),
    ])
    expect(out.map((s) => s.title)).toEqual(['alpha', 'Middle', 'zulu'])
  })

  it('numbers across every level, parent number first', () => {
    const out = buildOutline([
      doc('a', 'One', ''),
      doc('b', 'Two', ''),
      doc('c', 'Two', 'Two'),
      doc('d', 'Three', 'Two'),
      doc('e', 'Deep', 'Two/Two'),
      doc('f', 'Deeper', 'Two/Two/Deep'),
    ])
    expect(addresses(out)).toEqual([
      '1 One',
      '2 Two',
      '2.1 Three',
      '2.2 Two',
      '2.2.1 Deep',
      '2.2.1.1 Deeper',
    ])
  })

  it('nests as deep as the paths go', () => {
    const out = buildOutline([doc('a', 'Leaf', 'a/b/c/d/e')])
    const depths = flat(out).map((s) => s.depth)
    expect(depths).toEqual([0, 1, 2, 3, 4, 5])
    expect(flat(out).at(-1)!.number).toBe('1.1.1.1.1.1')
  })

  it('folds exactly one of two documents whose title collides with a sibling folder', () => {
    // Two documents called API in the same folder, and a folder called API. One
    // heads the folder; the other stays a leaf beside it. Which one is fixed by
    // id, so the rail does not reshuffle between reads.
    const out = buildOutline([
      doc('a2', 'API', 'root'),
      doc('a1', 'API', 'root'),
      doc('c', 'Child', 'root/API'),
    ])
    const root = out[0]!
    const heads = root.children.filter((s) => s.folder !== null)
    expect(heads).toHaveLength(1)
    expect(heads[0]!.doc?.id).toBe('a1')
    expect(root.children.filter((s) => s.folder === null).map((s) => s.doc?.id)).toEqual(['a2'])
  })

  it('does not let a document head a folder it is not a sibling of', () => {
    // `API` lives at the top level; the folder `Payments/API` is a level down.
    // Folding those would move a document into a branch it was never filed in.
    const out = buildOutline([doc('a', 'API', ''), doc('b', 'Thing', 'Payments/API')])
    const top = out.map((s) => ({ title: s.title, doc: s.doc?.id ?? null }))
    expect(top).toEqual([
      { title: 'API', doc: 'a' },
      { title: 'Payments', doc: null },
    ])
    expect(out[0]!.children).toEqual([])
  })

  it('normalises stray separators rather than growing a nameless folder', () => {
    const out = buildOutline([doc('a', 'Rates', '/product//billing/ ')])
    expect(addresses(out)).toEqual(['1 product', '1.1 billing', '1.1.1 Rates'])
  })

  it('shows every document exactly once, whatever the shape', () => {
    // The invariant the folding rule exists to keep. A document is either a
    // folder's head or a leaf, never both and never neither.
    const docs = [
      doc('a', 'API', 'Payments'),
      doc('b', 'API', 'Payments'),
      doc('c', 'Payments', ''),
      doc('d', 'Versioning', 'Payments/API'),
      doc('e', 'Loose', ''),
      doc('f', 'Hand filed', 'unnamed/folder'),
      doc('g', 'Payments', 'Payments'),
      doc('h', 'Deep', 'Payments/API/Versioning'),
    ]
    const seen = flat(buildOutline(docs))
      .map((s) => s.doc?.id)
      .filter((id): id is string => id !== undefined && id !== null)
    expect([...seen].sort()).toEqual(docs.map((d) => d.id).sort())
    expect(new Set(seen).size).toBe(docs.length)
  })

  it('gives every section a key unique across the whole outline', () => {
    const out = buildOutline([
      doc('a', 'API', 'Payments'),
      doc('b', 'Versioning', 'Payments/API'),
      doc('c', 'Loose', ''),
    ])
    const keys = flat(out).map((s) => s.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('is empty for no documents', () => {
    expect(buildOutline([])).toEqual([])
  })
})

describe('sectionCount', () => {
  it('counts every section beneath one, at any depth', () => {
    const out = buildOutline([
      doc('a', 'Root', ''),
      doc('b', 'Mid', 'Root'),
      doc('c', 'Leaf', 'Root/Mid'),
      doc('d', 'Other', 'Root'),
    ])
    expect(sectionCount(out[0]!)).toBe(3)
    expect(sectionCount(out[0]!.children[0]!)).toBe(1)
  })
})

describe('visibleSections', () => {
  it('hides everything beneath a folded section, at any depth', () => {
    const out = buildOutline([
      doc('a', 'Root', ''),
      doc('b', 'Mid', 'Root'),
      doc('c', 'Leaf', 'Root/Mid'),
      doc('d', 'Second', ''),
    ])
    expect(visibleSections(out, new Set(['a'])).map((s) => s.title)).toEqual(['Root', 'Second'])
    expect(visibleSections(out, new Set(['b'])).map((s) => s.title)).toEqual([
      'Root',
      'Mid',
      'Second',
    ])
  })
})

describe('ancestorKeys', () => {
  it('names every section that must be unfolded for one to be on screen', () => {
    const out = buildOutline([
      doc('a', 'Root', ''),
      doc('b', 'Mid', 'Root'),
      doc('c', 'Leaf', 'Root/Mid'),
    ])
    expect(ancestorKeys(out, 'c')).toEqual(['a', 'b'])
    expect(ancestorKeys(out, 'a')).toEqual([])
    expect(ancestorKeys(out, 'nope')).toEqual([])
  })
})
