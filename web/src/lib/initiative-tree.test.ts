import { describe, expect, it } from 'vitest'
import type { Initiative } from './initiatives'
import {
  MAX_DEPTH,
  ancestors,
  buildTree,
  folderPaths,
  normalizePath,
  pathOf,
  pruneTree,
  type TreeFolder,
} from './initiative-tree'

function doc(id: string, title: string, path?: string): Initiative {
  return {
    id,
    project: 'demo',
    title,
    status: 'open',
    ...(path === undefined ? {} : { metadata: { path } }),
  }
}

/** Child names of a folder, in render order. */
function names(f: TreeFolder): string[] {
  return f.children.map((c) => (c.kind === 'folder' ? c.name + '/' : c.initiative.title))
}

function folder(f: TreeFolder, name: string): TreeFolder {
  const hit = f.children.find((c) => c.kind === 'folder' && c.name === name)
  expect(hit, `expected a folder named ${name}`).toBeTruthy()
  return hit as TreeFolder
}

describe('normalizePath', () => {
  it('keeps an ordinary nested path', () => {
    expect(normalizePath('product/billing')).toBe('product/billing')
  })

  it('trims, and drops empty and traversal segments', () => {
    expect(normalizePath(' / product / /billing/ ')).toBe('product/billing')
    expect(normalizePath('../../etc/passwd')).toBe('etc/passwd')
    expect(normalizePath('a/./b')).toBe('a/b')
  })

  it('treats a non-string as unfiled rather than throwing', () => {
    expect(normalizePath(undefined)).toBe('')
    expect(normalizePath(42)).toBe('')
    expect(normalizePath({ path: 'a' })).toBe('')
  })

  it('folds depth beyond the cap into the last segment so nothing is unreachable', () => {
    const deep = Array.from({ length: MAX_DEPTH + 3 }, (_, i) => `l${i}`).join('/')
    const out = normalizePath(deep)
    expect(out.split('/')).toHaveLength(MAX_DEPTH)
    expect(out.endsWith('l7 l8 l9 l10')).toBe(true)
  })

  it('truncates an absurdly long segment', () => {
    expect(normalizePath('x'.repeat(200)).length).toBe(48)
  })
})

describe('pathOf', () => {
  it('reads and normalises the path off metadata', () => {
    expect(pathOf(doc('a', 'A', ' product/billing '))).toBe('product/billing')
  })

  it('files an initiative with no metadata at the root', () => {
    expect(pathOf(doc('a', 'A'))).toBe('')
    expect(pathOf({ ...doc('a', 'A'), metadata: 'nonsense' })).toBe('')
  })
})

describe('ancestors', () => {
  it('lists each ancestor path root-first', () => {
    expect(ancestors('a/b/c')).toEqual(['a', 'a/b', 'a/b/c'])
  })

  it('has none at the root', () => {
    expect(ancestors('')).toEqual([])
  })
})

describe('buildTree', () => {
  const items = [
    doc('i1', 'Pricing', 'product/billing'),
    doc('i2', 'Invoices', 'product/billing'),
    doc('i3', 'Onboarding', 'product'),
    doc('i4', 'Auth', 'platform'),
    doc('i5', 'Loose thought'),
  ]

  it('nests documents under the folders they name', () => {
    const root = buildTree(items)
    expect(names(root)).toEqual(['platform/', 'product/', 'Loose thought'])
    expect(names(folder(root, 'product'))).toEqual(['billing/', 'Onboarding'])
    expect(names(folder(folder(root, 'product'), 'billing'))).toEqual(['Invoices', 'Pricing'])
  })

  it('counts every document beneath a folder, not only its direct children', () => {
    const root = buildTree(items)
    expect(root.count).toBe(5)
    expect(folder(root, 'product').count).toBe(3)
    expect(folder(folder(root, 'product'), 'billing').count).toBe(2)
    expect(folder(root, 'platform').count).toBe(1)
  })

  it('creates intermediate folders nobody filed a document directly into', () => {
    const root = buildTree([doc('i1', 'Deep', 'a/b/c')])
    expect(names(root)).toEqual(['a/'])
    expect(names(folder(folder(root, 'a'), 'b'))).toEqual(['c/'])
    expect(folder(root, 'a').count).toBe(1)
  })

  it('is empty, not broken, with no documents', () => {
    const root = buildTree([])
    expect(root.children).toEqual([])
    expect(root.count).toBe(0)
  })

  it('sorts folders before documents, each case-insensitively', () => {
    const root = buildTree([doc('i1', 'zeta'), doc('i2', 'Alpha'), doc('i3', 'x', 'later')])
    expect(names(root)).toEqual(['later/', 'Alpha', 'zeta'])
  })
})

describe('folderPaths', () => {
  it('lists every folder as a full path, sorted', () => {
    const root = buildTree([doc('i1', 'A', 'product/billing'), doc('i2', 'B', 'platform')])
    expect(folderPaths(root)).toEqual(['platform', 'product', 'product/billing'])
  })
})

describe('pruneTree', () => {
  const root = buildTree([
    doc('i1', 'Pricing', 'product/billing'),
    doc('i2', 'Invoices', 'product/billing'),
    doc('i3', 'Auth', 'platform'),
  ])

  it('keeps the folders on the way to a survivor', () => {
    const hit = pruneTree(root, (i) => i.title === 'Pricing')
    expect(names(hit)).toEqual(['product/'])
    expect(names(folder(folder(hit, 'product'), 'billing'))).toEqual(['Pricing'])
    expect(hit.count).toBe(1)
  })

  it('drops folders left with nothing in them', () => {
    const hit = pruneTree(root, (i) => i.title === 'Auth')
    expect(names(hit)).toEqual(['platform/'])
  })

  it('returns an empty tree when nothing matches', () => {
    const hit = pruneTree(root, () => false)
    expect(hit.children).toEqual([])
    expect(hit.count).toBe(0)
  })

  it('does not mutate the tree it filtered', () => {
    pruneTree(root, () => false)
    expect(root.count).toBe(3)
    expect(names(root)).toEqual(['platform/', 'product/'])
  })
})
