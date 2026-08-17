// What "waiting" means, in one place.
//
// Two surfaces read this now — the explorer's per-row badges and the nav rail's
// count on /board — so the rule has to be shared rather than re-derived. A board
// badge that disagreed with the page it links to is worse than no badge: it sends
// someone looking for work that is not there, and they stop trusting the number.
import { describe, it, expect } from 'vitest'
import { countWaiting, isWaiting, waiting, type Rollup } from './initiatives'

const r = (over: Partial<Rollup> = {}): { rollup: Rollup } => ({
  rollup: { entries: 5, ...over },
})

describe('waiting', () => {
  it('reads both counts, defaulting a missing pair to zero', () => {
    expect(waiting({ open_notes: 2, pending_amendments: 1 })).toEqual({ notes: 2, amendments: 1 })
    expect(waiting({ entries: 9 })).toEqual({ notes: 0, amendments: 0 })
    expect(waiting(undefined)).toEqual({ notes: 0, amendments: 0 })
  })
})

describe('isWaiting', () => {
  it('is true when either kind is waiting, false when neither is', () => {
    expect(isWaiting(r({ open_notes: 1 }))).toBe(true)
    expect(isWaiting(r({ pending_amendments: 1 }))).toBe(true)
    expect(isWaiting(r({ open_notes: 0, pending_amendments: 0 }))).toBe(false)
  })

  // A server that predates the counts sends neither, and reading that as "quiet"
  // is the honest degradation: inventing attention from a missing field would
  // send readers into documents with nothing in them.
  it('treats an absent pair as quiet rather than unknown', () => {
    expect(isWaiting(r())).toBe(false)
    expect(isWaiting({})).toBe(false)
  })
})

describe('countWaiting', () => {
  it('counts documents, not notes', () => {
    // One document with nine open notes is still ONE place to go. Summing the
    // notes would make a single busy document look like a backlog.
    expect(countWaiting([r({ open_notes: 9, pending_amendments: 3 })])).toBe(1)
  })

  it('counts each waiting document once, however it is waiting', () => {
    expect(
      countWaiting([
        r({ open_notes: 2 }),
        r({ pending_amendments: 1 }),
        r({ open_notes: 1, pending_amendments: 1 }),
        r({ open_notes: 0, pending_amendments: 0 }),
        r(),
      ]),
    ).toBe(3)
  })

  it('is zero for an empty collection', () => {
    expect(countWaiting([])).toBe(0)
  })
})
