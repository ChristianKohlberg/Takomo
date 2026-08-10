import { describe, expect, it } from 'vitest'
import { clearedFilters, readView, writeView, type InboxView } from './inbox-url'

describe('readView / writeView', () => {
  it('an unfiltered inbox has a clean URL', () => {
    expect(writeView({ folder: 'open', group: false })).toBe('')
  })

  it('round-trips every filter', () => {
    const v: InboxView = {
      folder: 'answered',
      group: true,
      ticket: 'TK-1',
      search: 'billing rounding',
      urgency: ['critical', 'high'],
      mode: 'blocking',
      mine: true,
      hideAwaitingAgent: true,
      expiringSoon: true,
      askedBy: 'agent:runner-2',
    }
    expect(readView(writeView(v))).toEqual(v)
  })

  it('falls back rather than rendering a dead view', () => {
    // A stale or hand-edited link should open the inbox, not a folder that does
    // not exist and a rail with nothing selected.
    expect(readView('?folder=nonsense').folder).toBe('open')
    expect(readView('?mode=maybe').mode).toBeUndefined()
    expect(readView('?urgency=critical,made-up').urgency).toEqual(['critical'])
    expect(readView('?urgency=made-up').urgency).toBeUndefined()
  })

  it('reads the truthy spellings a hand-written link is likely to use', () => {
    expect(readView('?mine=1').mine).toBe(true)
    expect(readView('?mine=true').mine).toBe(true)
    expect(readView('?mine=0').mine).toBeUndefined()
  })

  it('clearing keeps the folder and the grouping — neither is a filter', () => {
    const v: InboxView = { folder: 'expired', group: true, ticket: 'TK-1', mine: true }
    expect(clearedFilters(v)).toEqual({ folder: 'expired', group: true })
  })
})
