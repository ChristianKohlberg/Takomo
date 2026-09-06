import { describe, expect, it } from 'vitest'
import { bugsLink, DEFAULT_BUGS_VIEW, readBugsView, writeBugsView, type BugsView } from './bugs-url'

describe('readBugsView / writeBugsView', () => {
  it('an unfiltered queue has a clean URL', () => {
    expect(writeBugsView(DEFAULT_BUGS_VIEW).toString()).toBe('')
    expect(readBugsView('')).toEqual(DEFAULT_BUGS_VIEW)
  })
  it('round-trips the project, the selection and every filter', () => {
    const v: BugsView = { project: 'demo', view: 'ready_for_review', severity: 'high', search: 'receipt total', state: 'active', assignee: 'none', researchStatus: 'completed', offset: 100, bug: 'demo-7' }
    expect(readBugsView(writeBugsView(v))).toEqual(v)
    expect(readBugsView(`?${writeBugsView(v)}`)).toEqual(v)
  })
  it('falls back from a stale or hostile link instead of an empty view', () => {
    const v = readBugsView('?view=nonsense&severity=urgent&research=maybe&offset=-3&project=demo')
    expect(v).toEqual({ ...DEFAULT_BUGS_VIEW, project: 'demo' })
    expect(readBugsView('?offset=abc').offset).toBe(0)
    expect(readBugsView('?offset=73').offset).toBe(50)
  })
  it('builds a link that opens one project on one bug', () => {
    expect(bugsLink('demo')).toBe('/bugs?project=demo')
    expect(bugsLink('demo', 'demo-7')).toBe('/bugs?project=demo&bug=demo-7')
    expect(bugsLink('')).toBe('/bugs')
  })
})
