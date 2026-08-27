// What the epics view must get right, minus what jsdom cannot see.
//
// There is no layout engine here, so nothing below proves the grid LOOKS right —
// the responsive contract in web/README.md and the eslint rules own that. What
// these prove is the reasoning: which epics are called stalled, what a claim
// reads as, that the lanes an epic belongs to are named rather than shown as
// ids, preset definitions, filters, honest unknown last-activity, and that a row
// is a real button a keyboard can reach.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { EpicsView } from './EpicsView'
import type { RoadmapEpic } from '@/lib/roadmap'
import {
  applyEpicsGrid,
  matchesPreset,
  presetSort,
  PRESET_IDS,
} from './epicsGrid'

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
  colEpic: 'Epic',
  colState: 'State',
  colLanes: 'Initiatives',
  colProgress: 'Progress',
  colHolder: 'Holder',
  colLastActivity: 'Last activity',
  sortAscending: 'sorted ascending',
  sortDescending: 'sorted descending',
  sortNone: 'sortable',
  filters: 'Filters',
  filterStateCategory: 'Category',
  filterLane: 'Initiative',
  filterClaimed: 'Claim',
  filterAll: 'All',
  filterClaimedYes: 'Claimed',
  filterClaimedNo: 'Unclaimed',
  clearFilters: 'Clear filters',
  noMatchFilters: 'No epics match these filters.',
  presets: 'Quick views',
  presetRecentCreated: 'Recently created',
  presetNearlyComplete: 'Nearly complete',
  presetNotStarted: 'Not started',
  presetStalled: 'Stalled',
  presetAwaiting: 'Awaiting answer',
  presetUnclaimed: 'Unclaimed',
  presetFlagged: 'Flagged',
  unclaimed: 'unclaimed',
  lastActivityUnknown: 'unknown (unclaimed)',
  stalledMarker: 'stalled',
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
  last_activity_at: '2026-08-16T12:00:00.000Z',
  idle_seconds: 260_000,
}

function view(epics: RoadmapEpic[], onOpen = vi.fn(), titles: Record<string, string> = {}) {
  render(<EpicsView epics={epics} laneTitles={titles} onOpen={onOpen} labels={labels} />)
  return onOpen
}

describe('epicsGrid presets', () => {
  const epics: RoadmapEpic[] = [
    base,
    {
      ...base,
      id: 'demo-b',
      title: 'Old epic',
      done: 0,
      percent: 0,
      total: 5,
      state_category: 'todo',
    },
    {
      ...base,
      id: 'demo-c',
      title: 'Nearly done',
      done: 4,
      percent: 80,
      total: 5,
      state_category: 'in_progress',
    },
    { ...base, id: 'demo-d', claim, flags: ['open_with_all_children_done'] },
    { ...base, id: 'demo-e', awaiting_answer: 2 },
  ]

  it('defines every shipped preset id', () => {
    expect(PRESET_IDS).toEqual([
      'recentCreated',
      'nearlyComplete',
      'notStarted',
      'stalled',
      'awaiting',
      'unclaimed',
      'flagged',
    ])
  })

  it('nearlyComplete means 75–99% with work', () => {
    const nearly = epics[2]!
    expect(matchesPreset(nearly, 'nearlyComplete', 86_400)).toBe(true)
    expect(matchesPreset(epics[0]!, 'nearlyComplete', 86_400)).toBe(false)
    expect(matchesPreset({ ...base, percent: 100, done: 3, total: 3 }, 'nearlyComplete', 86_400)).toBe(
      false,
    )
  })

  it('notStarted means zero children completed', () => {
    expect(matchesPreset(epics[1]!, 'notStarted', 86_400)).toBe(true)
    expect(matchesPreset(epics[0]!, 'notStarted', 86_400)).toBe(false)
  })

  it('recentCreated sorts by creation index descending', () => {
    expect(presetSort('recentCreated')).toEqual({ key: 'creation', dir: 'desc' })
    const out = applyEpicsGrid(epics, { stateCategory: '', lane: '', claimed: 'all' }, {
      key: 'creation',
      dir: 'asc',
    }, 'recentCreated', 86_400)
    expect(out.map((e) => e.id)).toEqual(['demo-e', 'demo-d', 'demo-c', 'demo-b', 'demo-a'])
  })

  it('stalled preset uses the same idle threshold as epicAttention', () => {
    const fresh = { ...base, id: 'fresh', claim: { ...claim, idle_seconds: 3_600 } }
    expect(matchesPreset(fresh, 'stalled', 7_200)).toBe(false)
    expect(matchesPreset(epics[3]!, 'stalled', 86_400)).toBe(true)
  })
})

describe('EpicsView', () => {
  it('says the project has no epics rather than rendering an empty list', () => {
    view([])
    expect(screen.getByText(labels.empty)).toBeTruthy()
    expect(screen.queryByRole('button', { name: /billing/i })).toBeNull()
  })

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

  it('falls back to the lane id when its title is unknown', () => {
    view([{ ...base, initiatives: ['ini-zz'] }])
    expect(screen.getByText('ini-zz')).toBeTruthy()
  })

  it('reads an indefinite claim as having no expiry, not as a duration', () => {
    view([{ ...base, claim }])
    expect(screen.getByRole('table').textContent).toMatch(
      /held by agent:w1.*no expiry · idle 3d/,
    )
  })

  it('reads a bounded claim as its held-for duration', () => {
    view([{ ...base, claim: { ...claim, indefinite: false, idle_seconds: 120 } }])
    expect(screen.getByRole('table').textContent).toMatch(/held by agent:w1.*2h · idle 2m/)
  })

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

  it('shows honest unknown for last activity when the epic is unclaimed', () => {
    view([base])
    expect(screen.getByText(labels.lastActivityUnknown)).toBeTruthy()
    expect(screen.queryByText('—')).toBeNull()
  })

  it('shows last activity age when a claim carries a timestamp', () => {
    const now = new Date('2026-08-17T12:00:00.000Z').getTime()
    vi.spyOn(Date, 'now').mockReturnValue(now)
    view([{ ...base, claim }])
    expect(screen.getByText('1d')).toBeTruthy()
    vi.restoreAllMocks()
  })

/**
 * Choose an option from a `Picker`.
 *
 * A native `<select>` took `fireEvent.change`; a listbox has to be opened first,
 * and Radix opens on `pointerDown`, not `click`. The stubs that let it open at
 * all live in src/test-setup.ts.
 */
function choose(labelText: string, optionName: string | RegExp) {
  const trigger = screen.getByLabelText(labelText)
  // Keyboard, not pointer: Radix's trigger opens on ArrowDown/Enter/Space, and
  // that path needs none of the geometry jsdom cannot supply.
  fireEvent.keyDown(trigger, { key: 'ArrowDown' })
  fireEvent.click(screen.getByRole('option', { name: optionName }))
}

  it('filters epics by claim state', () => {
    view([
      base,
      { ...base, id: 'demo-b', title: 'Claimed epic', claim },
    ])
    choose(labels.filterClaimed, labels.filterClaimedNo)
    expect(screen.getByRole('button', { name: /billing v2/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /claimed epic/i })).toBeNull()
  })

  it('filters epics by initiative lane', () => {
    view(
      [
        { ...base, initiatives: ['ini-a'] },
        { ...base, id: 'demo-b', title: 'Other', initiatives: ['ini-b'] },
      ],
      vi.fn(),
      { 'ini-a': 'Billing', 'ini-b': 'Reporting' },
    )
    choose(labels.filterLane, 'Billing')
    expect(screen.getByRole('button', { name: /billing v2/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /other/i })).toBeNull()
  })

  it('applies the unclaimed preset from the quick views', () => {
    view([
      base,
      { ...base, id: 'demo-b', title: 'Claimed epic', claim },
    ])
    fireEvent.click(screen.getByRole('button', { name: labels.presetUnclaimed }))
    expect(screen.getByRole('button', { name: /billing v2/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /claimed epic/i })).toBeNull()
  })

  it('labels recently created precisely — creation order, not activity', () => {
    view([
      { ...base, id: 'first', title: 'First' },
      { ...base, id: 'second', title: 'Second' },
    ])
    expect(screen.getByRole('button', { name: labels.presetRecentCreated })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: labels.presetRecentCreated }))
    const epicRows = screen
      .getAllByRole('button')
      .filter((b) => b.textContent?.includes('First') || b.textContent?.includes('Second'))
    expect(epicRows[0]?.textContent).toContain('Second')
  })
})
