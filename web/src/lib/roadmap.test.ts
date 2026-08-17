import { describe, expect, it } from 'vitest'
import {
  epicAttention,
  lane,
  laneTitles,
  laneVersions,
  laneWarnings,
  type Roadmap,
} from './roadmap'

const counts = { total: 0, done: 0, percent: 0, ready: 0, backlog: 0, awaiting_answer: 0 }

function epic(id: string, title: string, over: Partial<Roadmap['epics'][0]> = {}) {
  return {
    ...counts,
    id,
    title,
    state: 'ready',
    state_category: 'todo',
    priority: 'normal',
    flags: [],
    ...over,
  }
}

const rm: Roadmap = {
  project: 'demo',
  generated_at: '2026-08-17T00:00:00.000Z',
  epics: [
    epic('d-v1', 'Billing v1', { percent: 100, total: 2, done: 2 }),
    epic('d-other', 'Unrelated'),
    epic('d-v2', 'Billing v2', { percent: 33, total: 3, done: 1 }),
  ],
  initiatives: [
    { ...counts, id: 'ini-a', title: 'Billing', status: 'open', epics: ['d-v1', 'd-v2'], flags: [] },
    { ...counts, id: 'ini-b', title: 'Empty', status: 'open', epics: [], flags: ['empty_initiative'] },
  ],
}

describe('lane', () => {
  it('finds an initiative by id', () => {
    expect(lane(rm, 'ini-b')?.title).toBe('Empty')
  })

  it('is undefined for an unknown id, and for a filtered response with no lanes', () => {
    expect(lane(rm, 'ini-zz')).toBeUndefined()
    expect(lane({ ...rm, initiatives: undefined }, 'ini-a')).toBeUndefined()
    expect(lane(undefined, 'ini-a')).toBeUndefined()
  })
})

describe('laneVersions', () => {
  // The join is the point: a lane names ids, the epics carry the numbers.
  it('resolves a lane’s ids to full epic rollups', () => {
    const vs = laneVersions(rm, 'ini-a')
    expect(vs.map((e) => e.id)).toEqual(['d-v1', 'd-v2'])
    expect(vs.map((e) => e.percent)).toEqual([100, 33])
  })

  // Creation order, which for versions is the order they were planned in — NOT
  // the order the epics array happens to be in, where an unrelated epic sits
  // between them.
  it('keeps the lane’s order rather than the epics array’s', () => {
    const shuffled: Roadmap = { ...rm, epics: [...rm.epics].reverse() }
    expect(laneVersions(shuffled, 'ini-a').map((e) => e.id)).toEqual(['d-v1', 'd-v2'])
  })

  it('drops an id with no matching epic instead of yielding a blank row', () => {
    const stale: Roadmap = {
      ...rm,
      initiatives: [{ ...counts, id: 'ini-a', title: 'Billing', status: 'open', epics: ['d-v1', 'd-gone'], flags: [] }],
    }
    expect(laneVersions(stale, 'ini-a').map((e) => e.id)).toEqual(['d-v1'])
  })

  it('is empty for a lane with no versions, and for an absent roadmap', () => {
    expect(laneVersions(rm, 'ini-b')).toEqual([])
    expect(laneVersions(undefined, 'ini-a')).toEqual([])
  })
})

describe('laneWarnings', () => {
  it('surfaces a parked lane the queue is still feeding', () => {
    const l = { ...counts, id: 'x', title: 'X', status: 'parked', epics: [], flags: ['parked_with_ready_work'] }
    expect(laneWarnings(l)).toEqual(['parked_with_ready_work'])
  })

  // A lane opened before any work is filed is legitimate, and badging it as a
  // contradiction would make the ordinary way to start one look like a mistake.
  it('does not surface empty_initiative', () => {
    expect(laneWarnings(rm.initiatives![1])).toEqual([])
  })

  it('is empty for no lane', () => {
    expect(laneWarnings(undefined)).toEqual([])
  })
})

describe('laneTitles', () => {
  it('maps lane id to title, and is empty when a response carries no lanes', () => {
    expect(laneTitles(rm)).toEqual({ 'ini-a': 'Billing', 'ini-b': 'Empty' })
    expect(laneTitles({ ...rm, initiatives: undefined })).toEqual({})
    expect(laneTitles(undefined)).toEqual({})
  })
})

describe('epicAttention', () => {
  const claim = {
    holder: 'agent:w1',
    held_since: '2026-08-01T00:00:00.000Z',
    held_for_seconds: 100,
    indefinite: true,
    expires_at: null,
    last_activity_at: null,
    idle_seconds: 100,
  }

  it('counts nothing for an unheld, unflagged project', () => {
    expect(epicAttention([epic('a', 'A'), epic('b', 'B')])).toEqual({
      held: 0,
      stalled: 0,
      awaiting: 0,
      flagged: 0,
    })
  })

  // The number that only exists because an epic claim need not expire: nothing
  // will lapse and hand this back on its own.
  it('counts a held epic as stalled once nothing has moved for long enough', () => {
    const fresh = epic('a', 'A', { claim })
    const cold = epic('b', 'B', { claim: { ...claim, idle_seconds: 200_000 } })
    expect(epicAttention([fresh, cold])).toMatchObject({ held: 2, stalled: 1 })
  })

  it('takes the threshold as a parameter rather than burying it', () => {
    const e = epic('a', 'A', { claim: { ...claim, idle_seconds: 3_600 } })
    expect(epicAttention([e]).stalled).toBe(0)
    expect(epicAttention([e], 1_800).stalled).toBe(1)
  })

  // A claim with no idle number is held but not evidence of being stuck.
  it('does not call a claim with no idle reading stalled', () => {
    const e = epic('a', 'A', { claim: { ...claim, idle_seconds: null } })
    expect(epicAttention([e])).toMatchObject({ held: 1, stalled: 0 })
  })

  it('counts open questions and flags per epic, not per occurrence', () => {
    const q = epic('a', 'A', { awaiting_answer: 3 })
    const f = epic('b', 'B', { flags: ['done_with_open_children', 'empty_epic'] })
    expect(epicAttention([q, f])).toMatchObject({ awaiting: 1, flagged: 1 })
  })
})
