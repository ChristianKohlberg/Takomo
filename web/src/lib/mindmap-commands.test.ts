import { describe, expect, it } from 'vitest'

import {
  commandsFor,
  fuzzyRank,
  isTextEntry,
  matchScore,
  type CommandContext,
  type CommandNode,
} from './mindmap-commands'

const node = (over: Partial<CommandNode> = {}): CommandNode => ({
  id: 'mn-1',
  title: 'A thought',
  promoted: false,
  attachments: 0,
  hasChildren: false,
  collapsed: false,
  ...over,
})

const ctx = (over: Partial<CommandContext> = {}): CommandContext => ({
  canWrite: true,
  canManageMap: true,
  node: null,
  nodeCount: 4,
  projectCount: 3,
  ...over,
})

describe('commandsFor', () => {
  it('offers only map commands when nothing is selected', () => {
    expect(commandsFor(ctx())).toEqual([
      'map.goto',
      'map.fit',
      'map.tidy',
      'map.rename',
      'map.project',
      'map.delete',
    ])
  })

  it('puts the node scope first when a node is selected', () => {
    const ids = commandsFor(ctx({ node: node({ hasChildren: true }) }))
    expect(ids.slice(0, 10)).toEqual([
      'node.child',
      'node.sibling',
      'node.rename',
      'node.notes',
      'node.relate',
      'node.attach',
      'node.promoteEpic',
      'node.promoteInitiative',
      'node.collapse',
      'node.delete',
    ])
    expect(ids).toContain('map.fit')
  })

  it('hides what does not apply rather than offering it disabled', () => {
    // A read-only token can look and navigate. Nothing it would be refused for
    // appears at all.
    const ids = commandsFor(
      ctx({ canWrite: false, canManageMap: false, node: node({ hasChildren: true }) }),
    )
    // Switching project is a read, so it survives; everything that would write
    // is absent.
    expect(ids).toEqual(['node.collapse', 'map.goto', 'map.fit', 'map.project'])
  })

  it('does not offer to promote a branch that already graduated', () => {
    const ids = commandsFor(ctx({ node: node({ promoted: true }) }))
    expect(ids).not.toContain('node.promoteEpic')
    expect(ids).not.toContain('node.promoteInitiative')
  })

  it('stops offering attachments at the cap', () => {
    expect(commandsFor(ctx({ node: node({ attachments: 19 }) }))).toContain('node.attach')
    expect(commandsFor(ctx({ node: node({ attachments: 20 }) }))).not.toContain('node.attach')
  })

  it('offers exactly one of collapse and expand, and neither for a leaf', () => {
    expect(commandsFor(ctx({ node: node({ hasChildren: true }) }))).toContain('node.collapse')
    expect(commandsFor(ctx({ node: node({ hasChildren: true, collapsed: true }) }))).toContain(
      'node.expand',
    )
    const leaf = commandsFor(ctx({ node: node() }))
    expect(leaf).not.toContain('node.collapse')
    expect(leaf).not.toContain('node.expand')
  })

  it('needs a second node before a relation is drawable', () => {
    expect(commandsFor(ctx({ nodeCount: 1, node: node() }))).not.toContain('node.relate')
  })

  it('does not offer to switch project when there is only one', () => {
    expect(commandsFor(ctx({ projectCount: 1 }))).not.toContain('map.project')
  })

  it('offers nothing to navigate or tidy on an empty map', () => {
    const ids = commandsFor(ctx({ nodeCount: 0 }))
    expect(ids).not.toContain('map.goto')
    expect(ids).not.toContain('map.tidy')
    expect(ids).toContain('map.fit')
  })
})

describe('matchScore', () => {
  it('ranks an exact title above a prefix above a word start above a substring', () => {
    const exact = matchScore('billing', 'billing')!
    const prefix = matchScore('billing rules', 'billing')!
    const word = matchScore('rewrite billing rules', 'billing')!
    const mid = matchScore('rebilling rules', 'billing')!
    expect(exact).toBeLessThan(prefix)
    expect(prefix).toBeLessThan(word)
    expect(word).toBeLessThan(mid)
  })

  it('matches a subsequence, and ranks it below every literal match', () => {
    const sub = matchScore('Broaden the redesign', 'brd')!
    expect(sub).not.toBeNull()
    expect(matchScore('brd rules', 'brd')!).toBeLessThan(sub)
  })

  it('prefers a tight subsequence to a scattered one', () => {
    const tight = matchScore('abc later', 'abc')!
    const loose = matchScore('a big careful thing', 'abc')!
    expect(tight).toBeLessThan(loose)
  })

  it('refuses a query whose characters are out of order', () => {
    expect(matchScore('abc', 'cba')).toBeNull()
    expect(matchScore('billing', 'zz')).toBeNull()
  })

  it('is case-insensitive and ignores surrounding space', () => {
    expect(matchScore('Billing', '  billing ')).toBe(0)
  })
})

describe('fuzzyRank', () => {
  const titles = ['Payments', 'Rewrite billing rules', 'Billing', 'Bring the docs', 'unrelated']
  const rank = (q: string, max = 10) => fuzzyRank(titles, (t) => t, q, max)

  it('keeps the given order when nothing is typed', () => {
    expect(rank('')).toEqual(titles)
  })

  it('puts the exact title first, then the prefix, then the word start', () => {
    expect(rank('billing')).toEqual(['Billing', 'Rewrite billing rules'])
  })

  it('falls back to a subsequence rather than saying nothing matches', () => {
    expect(rank('brg')).toContain('Bring the docs')
  })

  it('truncates without reordering', () => {
    expect(rank('', 2)).toEqual(['Payments', 'Rewrite billing rules'])
  })

  it('keeps the caller order between equally good matches', () => {
    expect(fuzzyRank(['ab one', 'ab two'], (t) => t, 'ab', 10)).toEqual(['ab one', 'ab two'])
  })
})

describe('isTextEntry', () => {
  it('recognises the elements ⌘K must not open over', () => {
    expect(isTextEntry({ tagName: 'INPUT' })).toBe(true)
    expect(isTextEntry({ tagName: 'textarea' })).toBe(true)
    expect(isTextEntry({ tagName: 'SELECT' })).toBe(true)
    expect(isTextEntry({ tagName: 'DIV', isContentEditable: true })).toBe(true)
  })

  it('leaves the canvas and the page body alone', () => {
    expect(isTextEntry({ tagName: 'svg' })).toBe(false)
    expect(isTextEntry({ tagName: 'BODY' })).toBe(false)
    expect(isTextEntry(null)).toBe(false)
  })
})
