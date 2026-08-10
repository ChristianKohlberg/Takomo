import { describe, expect, it } from 'vitest'
import {
  activeFilterCount,
  filterQuestions,
  groupByEpic,
  isExpiringSoon,
  matchesSearch,
  sortForFolder,
} from './question-filters'
import { indexById } from './tickets'
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
    expect(activeFilterCount({ urgency: [] })).toBe(0)
    expect(activeFilterCount({ ticket: 'TK-1', search: 'x' })).toBe(2)
    expect(
      activeFilterCount({ urgency: ['critical'], mode: 'blocking', mine: true, expiringSoon: true }),
    ).toBe(4)
  })
})

// The tree the inbox now carries so it can filter by an epic and group by one.
//
//   EP-1 (epic) ── TK-1
//               └─ TK-2
//   TK-3 (no epic)
const TREE = indexById([
  { id: 'EP-1', type: 'epic', title: 'Billing rewrite' },
  { id: 'TK-1', parent: 'EP-1' },
  { id: 'TK-2', parent: 'EP-1' },
  { id: 'TK-3' },
])

describe('the ticket filter, with a tree', () => {
  const all = [q({ id: 'a', ticket: 'TK-1' }), q({ id: 'b', ticket: 'TK-3' })]

  it('filtering by an epic keeps the questions on its children', () => {
    // Without the walk this is empty: questions hang off the LEAVES, so an
    // exact match on an epic shows an inbox that looks like nobody has asked
    // anything about it.
    expect(filterQuestions(all, { ticket: 'EP-1' }, { index: TREE }).map((x) => x.id)).toEqual(['a'])
  })

  it('falls back to an exact match with no index', () => {
    expect(filterQuestions(all, { ticket: 'EP-1' })).toEqual([])
    expect(filterQuestions(all, { ticket: 'TK-1' }).map((x) => x.id)).toEqual(['a'])
  })

  it('tolerates a question on a ticket the index has never heard of', () => {
    const orphan = [q({ id: 'z', ticket: 'GONE-9' })]
    expect(filterQuestions(orphan, { ticket: 'EP-1' }, { index: TREE })).toEqual([])
    expect(filterQuestions(orphan, { ticket: 'GONE-9' }, { index: TREE }).map((x) => x.id)).toEqual([
      'z',
    ])
  })
})

describe('the rest of the filters', () => {
  it('treats an absent urgency as normal, the way the row does', () => {
    const list = [q({ id: 'a', urgency: 'critical' }), q({ id: 'b' })]
    expect(filterQuestions(list, { urgency: ['normal'] }).map((x) => x.id)).toEqual(['b'])
    expect(filterQuestions(list, { urgency: ['critical', 'normal'] })).toHaveLength(2)
  })

  it('splits blocking from advisory', () => {
    const list = [q({ id: 'a', mode: 'blocking' }), q({ id: 'b', mode: 'advisory' })]
    expect(filterQuestions(list, { mode: 'advisory' }).map((x) => x.id)).toEqual(['b'])
  })

  it('"mine" is expertise overlap, and empty expertise matches nothing', () => {
    const list = [q({ id: 'a', expertise: ['domain:billing'] }), q({ id: 'b' })]
    expect(
      filterQuestions(list, { mine: true }, { expertise: ['domain:billing'] }).map((x) => x.id),
    ).toEqual(['a'])
    expect(filterQuestions(list, { mine: true }, { expertise: [] })).toEqual([])
  })

  it('hides questions bounced back to the agent', () => {
    const list = [q({ id: 'a', awaiting: 'agent' }), q({ id: 'b', awaiting: 'human' })]
    expect(filterQuestions(list, { hideAwaitingAgent: true }).map((x) => x.id)).toEqual(['b'])
  })

  it('"expiring soon" excludes questions that never expire', () => {
    const now = Date.parse('2026-01-01T00:00:00Z')
    const soon = q({ id: 'a', expires_at: '2026-01-01T06:00:00Z' })
    const later = q({ id: 'b', expires_at: '2026-01-09T00:00:00Z' })
    const never = q({ id: 'c' })
    expect(isExpiringSoon(soon, now)).toBe(true)
    expect(isExpiringSoon(never, now)).toBe(false)
    expect(
      filterQuestions([soon, later, never], { expiringSoon: true }, { now }).map((x) => x.id),
    ).toEqual(['a'])
  })

  it('matches an asker exactly', () => {
    const list = [q({ id: 'a', asked_by: 'agent:runner-1' }), q({ id: 'b', asked_by: 'agent:x' })]
    expect(filterQuestions(list, { askedBy: 'agent:runner-1' }).map((x) => x.id)).toEqual(['a'])
  })
})

describe('sortForFolder', () => {
  const list = [
    q({ id: 'old', answered_at: '2026-01-01T00:00:00Z' }),
    q({ id: 'new', answered_at: '2026-06-01T00:00:00Z' }),
  ]

  it('leaves Open in the server order — that IS the triage order', () => {
    expect(sortForFolder(list, 'open').map((x) => x.id)).toEqual(['old', 'new'])
  })

  it('puts the most recent decision first in every closed folder', () => {
    // The server orders answered questions by urgency and CREATION, so this
    // folder opened on the oldest decision anyone ever made.
    expect(sortForFolder(list, 'answered').map((x) => x.id)).toEqual(['new', 'old'])
  })

  it('does not mutate its input', () => {
    const before = list.map((x) => x.id)
    sortForFolder(list, 'answered')
    expect(list.map((x) => x.id)).toEqual(before)
  })
})

describe('groupByEpic', () => {
  it('groups by the epic above the question tickets, remainder last', () => {
    const groups = groupByEpic(
      [q({ id: 'a', ticket: 'TK-3' }), q({ id: 'b', ticket: 'TK-1' }), q({ id: 'c', ticket: 'TK-2' })],
      TREE,
    )
    expect(groups.map((g) => g.epic)).toEqual(['EP-1', ''])
    expect(groups[0]!.title).toBe('Billing rewrite')
    expect(groups[0]!.questions.map((x) => x.id)).toEqual(['b', 'c'])
    expect(groups[1]!.questions.map((x) => x.id)).toEqual(['a'])
  })

  it('keeps every question exactly once', () => {
    const list = [q({ id: 'a', ticket: 'TK-1' }), q({ id: 'b', ticket: 'GONE' })]
    const flat = groupByEpic(list, TREE).flatMap((g) => g.questions.map((x) => x.id))
    expect(flat.sort()).toEqual(['a', 'b'])
  })
})
