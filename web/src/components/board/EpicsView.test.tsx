// What the epics view must get right, minus what jsdom cannot see.
//
// There is no layout engine here, so nothing below proves the rows LOOK right —
// the responsive contract in web/README.md and the eslint rules own that. What
// these prove is the reasoning: which epics are called stalled, what a claim
// reads as, that the lanes an epic belongs to are named rather than shown as
// ids, and that a row is a real button a keyboard can reach.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { EpicsView } from './EpicsView'
import type { RoadmapEpic } from '@/lib/roadmap'

afterEach(cleanup)

const labels = {
  held: 'held',
  stalled: 'stalled',
  awaiting: 'awaiting an answer',
  flagged: 'flagged',
  ready: 'ready',
  backlog: 'backlog',
  heldBy: 'held by',
  idle: 'idle',
  indefinite: 'no expiry',
  noLane: 'No initiative',
  empty: 'No epics in this project.',
  emptyHint: 'An epic groups the work of one version.',
  progress: 'Progress',
}

const base: RoadmapEpic = {
  id: 'demo-a',
  title: 'Billing v2',
  state: 'ready',
  state_category: 'todo',
  priority: 'normal',
  total: 3,
  done: 1,
  percent: 33,
  ready: 2,
  backlog: 0,
  awaiting_answer: 0,
  initiatives: [],
  claim: null,
  flags: [],
}

const claim = {
  holder: 'agent:w1',
  held_since: '2026-08-14T00:00:00.000Z',
  held_for_seconds: 7200,
  indefinite: true,
  expires_at: null,
  last_activity_at: null,
  idle_seconds: 260_000,
}

function view(epics: RoadmapEpic[], onOpen = vi.fn(), titles: Record<string, string> = {}) {
  render(<EpicsView epics={epics} laneTitles={titles} onOpen={onOpen} labels={labels} />)
  return onOpen
}

describe('EpicsView', () => {
  it('says the project has no epics rather than rendering an empty list', () => {
    view([])
    expect(screen.getByText(labels.empty)).toBeTruthy()
    expect(screen.queryByRole('button')).toBeNull()
  })

  // A row has to be reachable and activatable by keyboard: a clickable <div>
  // would look identical and silently lose that.
  it('makes each epic a real button that opens the ticket', () => {
    const onOpen = view([base])
    const row = screen.getByRole('button', { name: /billing v2/i })
    fireEvent.click(row)
    expect(onOpen).toHaveBeenCalledWith('demo-a')
  })

  it('names the lanes an epic belongs to, and says so when it belongs to none', () => {
    view([{ ...base, initiatives: ['ini-a', 'ini-b'] }], vi.fn(), {
      'ini-a': 'Billing',
      'ini-b': 'Reporting',
    })
    expect(screen.getByText('Billing · Reporting')).toBeTruthy()
    cleanup()
    view([base])
    expect(screen.getByText(labels.noLane)).toBeTruthy()
  })

  // A lane id with no known title still has to render as something, or an epic
  // filed under a lane the reader cannot see reads as filed under nothing.
  it('falls back to the lane id when its title is unknown', () => {
    view([{ ...base, initiatives: ['ini-zz'] }])
    expect(screen.getByText('ini-zz')).toBeTruthy()
  })

  it('reads an indefinite claim as having no expiry, not as a duration', () => {
    view([{ ...base, claim }])
    expect(screen.getByText(/held by agent:w1 · no expiry · idle 3d/)).toBeTruthy()
  })

  it('reads a bounded claim as its held-for duration', () => {
    view([{ ...base, claim: { ...claim, indefinite: false, idle_seconds: 120 } }])
    expect(screen.getByText(/held by agent:w1 · 2h · idle 2m/)).toBeTruthy()
  })

  // The counted summary is the whole reason this view is faster to read than the
  // board: the answer arrives before the scrolling does.
  it('counts what wants attention across every epic', () => {
    view([
      { ...base, claim },
      { ...base, id: 'demo-b', awaiting_answer: 2 },
      { ...base, id: 'demo-c', flags: ['done_with_open_children'] },
    ])
    const strip = screen.getByText('held').parentElement?.parentElement
    expect(strip?.textContent).toContain('1 held')
    expect(strip?.textContent).toContain('1 stalled')
    expect(strip?.textContent).toContain('1 awaiting an answer')
    expect(strip?.textContent).toContain('1 flagged')
  })

  it('respects the stalled threshold it is given', () => {
    const fresh = { ...base, claim: { ...claim, idle_seconds: 3_600 } }
    render(
      <EpicsView
        epics={[fresh]}
        laneTitles={{}}
        onOpen={vi.fn()}
        stalledAfter={7_200}
        labels={labels}
      />,
    )
    const strip = screen.getByText('stalled').parentElement?.parentElement
    expect(strip?.textContent).toContain('0 stalled')
  })

  // A finished epic should not carry a row of zeroes — the counts are there to
  // be scanned, and zeroes are noise that hides the non-zero ones. The state is
  // `done` here so the state BADGE cannot be mistaken for a count label.
  it('omits claimable counts entirely when there are none', () => {
    view([
      {
        ...base,
        state: 'done',
        state_category: 'done',
        total: 2,
        done: 2,
        percent: 100,
        ready: 0,
        backlog: 0,
      },
    ])
    expect(screen.queryByText(/^ready /)).toBeNull()
    expect(screen.queryByText(/^backlog /)).toBeNull()
  })

  it('shows an epic’s flags so a contradiction is visible without opening it', () => {
    view([{ ...base, flags: ['open_with_all_children_done'] }])
    expect(screen.getByText('open_with_all_children_done')).toBeTruthy()
  })
})
