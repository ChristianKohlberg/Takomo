import { describe, expect, it } from 'vitest'

import { standingOf, traceActor, traceByNode } from './plan-trace'
import type { TraceEntry } from './mindmaps'

function entry(over: Partial<TraceEntry> = {}): TraceEntry {
  return {
    id: 'tr-1',
    node: 'mn-1',
    kind: 'edited',
    actor: 'agent:fleet-3',
    user: null,
    note: null,
    text: null,
    at: '2026-08-30T10:00:00.000Z',
    ...over,
  }
}

describe('standingOf', () => {
  it('reads a section nobody has ever reviewed as unseen', () => {
    expect(standingOf(undefined)).toBe('unseen')
    expect(standingOf(null)).toBe('unseen')
    expect(
      standingOf({ changed_at: '2026-08-30T10:00:00Z', reviewed_at: null, confirmed: false }),
    ).toBe('unseen')
  })

  it('reads a review that came after the last change as agreed', () => {
    expect(
      standingOf({
        changed_at: '2026-08-30T10:00:00Z',
        reviewed_at: '2026-08-30T11:00:00Z',
        confirmed: true,
      }),
    ).toBe('confirmed')
  })

  it('reads a section that moved after its review as changed, not unseen', () => {
    // The distinction the boolean cannot carry, and the reason this is a reading
    // rather than a stored flag: somebody agreed with an older version of this.
    expect(
      standingOf({
        changed_at: '2026-08-30T12:00:00Z',
        reviewed_at: '2026-08-30T11:00:00Z',
        confirmed: false,
      }),
    ).toBe('changed')
  })
})

describe('traceByNode', () => {
  it('groups a page of history by section, keeping the order it arrived in', () => {
    const entries = [
      entry({ id: 'a', node: 'mn-1', kind: 'edited' }),
      entry({ id: 'b', node: 'mn-2', kind: 'authored' }),
      entry({ id: 'c', node: 'mn-1', kind: 'authored' }),
    ]
    const grouped = traceByNode(entries)
    expect(grouped.get('mn-1')?.map((e) => e.id)).toEqual(['a', 'c'])
    expect(grouped.get('mn-2')?.map((e) => e.id)).toEqual(['b'])
  })

  it('leaves an entry about the plan as a whole out of every section', () => {
    const grouped = traceByNode([entry({ node: null })])
    expect(grouped.size).toBe(0)
  })
})

describe('traceActor', () => {
  it('names the person where the credential was bound to one', () => {
    const named = new Map([['usr-7', 'Ada']])
    expect(traceActor(entry({ user: 'usr-7' }), named)).toBe('Ada')
  })

  it('falls back to the user id, then to the actor string', () => {
    expect(traceActor(entry({ user: 'usr-7' }))).toBe('usr-7')
    expect(traceActor(entry({ user: null }))).toBe('agent:fleet-3')
  })
})
