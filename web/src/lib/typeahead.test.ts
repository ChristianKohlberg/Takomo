import { describe, expect, it } from 'vitest'
import { rankOptions } from './typeahead'

const OPTS = [
  { id: 'demo', title: 'Demo — agent fleet' },
  { id: 'billing', title: 'Billing platform' },
  { id: 'demo-two', title: null },
  { id: 'infra', title: 'Demo infrastructure' },
]

describe('rankOptions', () => {
  it('returns everything, in order, for an empty query', () => {
    const r = rankOptions(OPTS, '  ', 10)
    expect(r.all.map((o) => o.id)).toEqual(['demo', 'billing', 'demo-two', 'infra'])
    expect(r.total).toBe(4)
  })

  it('puts an exact id first, then an id prefix, then a title prefix', () => {
    // 'infra' matches only on its title, which starts with "demo" — it must
    // rank below both id matches rather than above them.
    const r = rankOptions(OPTS, 'demo', 10)
    expect(r.all.map((o) => o.id)).toEqual(['demo', 'demo-two', 'infra'])
  })

  it('matches on the title as well as the id', () => {
    expect(rankOptions(OPTS, 'platform', 10).all.map((o) => o.id)).toEqual(['billing'])
  })

  it('counts before truncating — the bug this function exists to prevent', () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ id: `p${i}`, title: null }))
    const r = rankOptions(many, 'p', 12)
    expect(r.shown).toHaveLength(12)
    // Not 12: a footer reading `total` says "12 of 40", not "40 matches".
    expect(r.total).toBe(40)
  })

  it('keeps server order within a rank so results do not shuffle as you type', () => {
    const same = [
      { id: 'aa1', title: null },
      { id: 'aa2', title: null },
      { id: 'aa3', title: null },
    ]
    expect(rankOptions(same, 'aa', 10).all.map((o) => o.id)).toEqual(['aa1', 'aa2', 'aa3'])
  })

  it('is case-insensitive on both fields', () => {
    expect(rankOptions(OPTS, 'BILLING', 10).total).toBe(1)
    expect(rankOptions(OPTS, 'AGENT', 10).all[0]?.id).toBe('demo')
  })

  it('returns nothing when nothing matches', () => {
    const r = rankOptions(OPTS, 'zzz', 10)
    expect(r.total).toBe(0)
    expect(r.shown).toEqual([])
  })
})
