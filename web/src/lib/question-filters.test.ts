import { describe, expect, it } from 'vitest'
import { activeFilterCount, filterQuestions, matchesSearch } from './question-filters'
import type { Question } from './questions'

function q(over: Partial<Question>): Question {
  return {
    id: 'q1',
    project: 'demo',
    ticket: 'TK-1',
    kind: 'confirm',
    mode: 'blocking',
    status: 'open',
    title: 'Ship the migration?',
    options: [],
    option_notes: [],
    multi: false,
    recommended_multi: [],
    expertise: [],
    created_at: '2026-01-01T00:00:00Z',
    ...over,
  }
}

describe('matchesSearch', () => {
  it('admits everything for an empty term', () => {
    expect(matchesSearch(q({}), '   ')).toBe(true)
  })

  it('is case-insensitive on the title', () => {
    expect(matchesSearch(q({}), 'MIGRATION')).toBe(true)
  })

  it('searches the body, which the row never renders', () => {
    const x = q({ title: 'Pick one', body: 'the invoice rounding rule' })
    expect(matchesSearch(x, 'rounding')).toBe(true)
  })

  it('searches the ticket, the asker, and expertise tags', () => {
    const x = q({ ticket: 'TK-42', asked_by: 'agent-7', expertise: ['domain:billing'] })
    expect(matchesSearch(x, 'tk-42')).toBe(true)
    expect(matchesSearch(x, 'agent-7')).toBe(true)
    expect(matchesSearch(x, 'billing')).toBe(true)
  })

  it('requires EVERY term — two words narrow, they do not widen', () => {
    const x = q({ title: 'Ship the migration?', asked_by: 'agent-7' })
    expect(matchesSearch(x, 'migration agent-7')).toBe(true)
    expect(matchesSearch(x, 'migration agent-9')).toBe(false)
  })
})

describe('filterQuestions', () => {
  const all = [
    q({ id: 'a', ticket: 'TK-1', title: 'Ship the migration?' }),
    q({ id: 'b', ticket: 'TK-2', title: 'Refund policy' }),
    q({ id: 'c', ticket: 'TK-2', title: 'Migration window' }),
  ]

  it('matches a ticket exactly, not as a prefix', () => {
    expect(filterQuestions(all, { ticket: 'TK-2' }).map((x) => x.id)).toEqual(['b', 'c'])
    expect(filterQuestions(all, { ticket: 'TK-' })).toEqual([])
  })

  it('composes ticket and search', () => {
    expect(filterQuestions(all, { ticket: 'TK-2', search: 'migration' }).map((x) => x.id)).toEqual([
      'c',
    ])
  })

  it('returns everything when nothing is set', () => {
    expect(filterQuestions(all, {})).toHaveLength(3)
    expect(filterQuestions(all, { search: '  ' })).toHaveLength(3)
  })
})

describe('activeFilterCount', () => {
  it('counts only filters that actually narrow', () => {
    expect(activeFilterCount({})).toBe(0)
    expect(activeFilterCount({ search: '   ' })).toBe(0)
    expect(activeFilterCount({ ticket: 'TK-1', search: 'x' })).toBe(2)
  })
})
