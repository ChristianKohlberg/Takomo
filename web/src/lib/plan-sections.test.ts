// The numbering, the fold and the shape guard.
//
// These tests moved here from `document-outline.test.ts` when the plan stopped
// being a tree of document rows and became the map's own tree. The subject
// changed — sections come from nodes, not from folder paths — but the three
// promises worth keeping are the same ones, and they are asserted the same way:
// the number is computed from position, a fold hides exactly what it holds, and
// a section can be found from anywhere above it.
import { describe, expect, it } from 'vitest'

import {
  ancestorKeys,
  flattenSections,
  planSections,
  sameTree,
  sectionCount,
  visibleSections,
  type PlanNode,
  type PlanSection,
} from './plan-sections'

/** Nodes as `readPlanTree` hands them over: depth-first, parents first. */
function node(id: string, parent: string | null, title: string, position = 0): PlanNode {
  return { id, parent, title, order: `a${position}`, position }
}

const label = (s: PlanSection): string => `${s.number} ${s.title}`
const addresses = (sections: readonly PlanSection[]): string[] =>
  flattenSections(sections).map(label)

/** A three-level plan, which is what every assertion below reads. */
const plan = [
  node('a', null, 'Payments rebuild', 0),
  node('b', 'a', 'API', 0),
  node('c', 'b', 'Versioning', 0),
  node('d', 'a', 'Billing', 1),
  node('e', null, 'Onboarding', 1),
]

describe('planSections', () => {
  it('numbers sections by their position in the tree', () => {
    expect(addresses(planSections(plan))).toEqual([
      '1 Payments rebuild',
      '1.1 API',
      '1.1.1 Versioning',
      '1.2 Billing',
      '2 Onboarding',
    ])
  })

  it('reads depth as heading level, from zero', () => {
    const flat = flattenSections(planSections(plan))
    expect(flat.map((s) => s.depth)).toEqual([0, 1, 2, 1, 0])
  })

  it('keys a section by its node, because a section IS a node', () => {
    expect(flattenSections(planSections(plan)).map((s) => s.key)).toEqual([
      'a',
      'b',
      'c',
      'd',
      'e',
    ])
  })

  it('keeps a section whose parent is not in the set, at the top level', () => {
    // A node can arrive before its parent does — the document is syncing, and
    // losing a whole branch until it catches up is worse than showing it.
    const out = planSections([node('x', 'missing', 'Orphan')])
    expect(addresses(out)).toEqual(['1 Orphan'])
  })

  it('trims a title but keeps an empty one, so the page can say "untitled"', () => {
    const out = planSections([node('a', null, '  Spaced  '), node('b', null, '   ', 1)])
    expect(out[0]?.title).toBe('Spaced')
    expect(out[1]?.title).toBe('')
  })

  it('draws each section once even if the tree it is handed loops', () => {
    // `normaliseNodes` breaks cycles before this runs; the guard is here because
    // a hung render is a worse answer than a repaired tree.
    const out = planSections([node('a', 'b', 'A'), node('b', 'a', 'B')])
    expect(flattenSections(out)).toHaveLength(2)
  })
})

describe('sectionCount', () => {
  it('counts everything beneath a section, at any depth', () => {
    const out = planSections(plan)
    expect(sectionCount(out[0] as PlanSection)).toBe(3)
    expect(sectionCount(out[1] as PlanSection)).toBe(0)
  })
})

describe('visibleSections', () => {
  it('hides what a folded section holds, and nothing else', () => {
    const out = planSections(plan)
    expect(visibleSections(out, new Set(['a'])).map(label)).toEqual([
      '1 Payments rebuild',
      '2 Onboarding',
    ])
  })

  it('leaves the numbers alone: a fold is a view, not a renumbering', () => {
    const out = planSections(plan)
    const rows = visibleSections(out, new Set(['b']))
    expect(rows.map((s) => s.number)).toEqual(['1', '1.1', '1.2', '2'])
  })
})

describe('ancestorKeys', () => {
  it('names every section that has to be unfolded to reach one', () => {
    expect(ancestorKeys(planSections(plan), 'c')).toEqual(['a', 'b'])
  })

  it('is empty for a top-level section, and for one that is not there', () => {
    expect(ancestorKeys(planSections(plan), 'a')).toEqual([])
    expect(ancestorKeys(planSections(plan), 'nope')).toEqual([])
  })
})

describe('sameTree', () => {
  it('is true when only the prose could have changed', () => {
    expect(sameTree(plan, [...plan])).toBe(true)
  })

  it('is false when a title, a parent, an order or the count moved', () => {
    expect(sameTree(plan, [...plan.slice(0, 4), node('e', null, 'Onboarding v2', 1)])).toBe(false)
    expect(sameTree(plan, [...plan.slice(0, 4), node('e', 'a', 'Onboarding', 1)])).toBe(false)
    expect(sameTree(plan, [...plan.slice(0, 4), node('e', null, 'Onboarding', 9)])).toBe(false)
    expect(sameTree(plan, plan.slice(0, 4))).toBe(false)
  })
})
