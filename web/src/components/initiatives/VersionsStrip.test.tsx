// The versions strip, minus what jsdom cannot see (layout — see web/README.md).
//
// What matters here is that an initiative with no work says so instead of
// rendering an empty box, that a lane spanning versions reports the lane's own
// total rather than a version's, and that the bar is a real progressbar a screen
// reader can read a number off.
import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import { VersionsStrip } from './VersionsStrip'
import type { RoadmapEpic, RoadmapLane } from '@/lib/roadmap'

afterEach(cleanup)

const labels = {
  heading: 'Versions',
  done: 'done',
  ready: 'ready',
  backlog: 'backlog',
  awaiting: 'awaiting an answer',
  empty: 'No versions filed yet.',
  emptyHint: 'Tag a ticket or epic with this initiative.',
  parkedWithReadyWork: 'Parked, but work is still being handed out',
}

const lane: RoadmapLane = {
  id: 'ini-a',
  title: 'Billing',
  status: 'open',
  total: 5,
  done: 3,
  percent: 60,
  ready: 2,
  backlog: 0,
  awaiting_answer: 1,
  epics: ['demo-v1', 'demo-v2'],
  flags: [],
}

function epic(over: Partial<RoadmapEpic> & { id: string; title: string }): RoadmapEpic {
  return {
    state: 'ready',
    state_category: 'todo',
    priority: 'normal',
    total: 0,
    done: 0,
    percent: 0,
    ready: 0,
    backlog: 0,
    awaiting_answer: 0,
    flags: [],
    ...over,
  }
}

describe('VersionsStrip', () => {
  // Absent is not empty: while the roadmap is still loading there is nothing
  // truthful to say, so the strip stays out of the document entirely.
  it('renders nothing at all without a lane', () => {
    const { container } = render(
      <VersionsStrip lane={undefined} versions={[]} labels={labels} />,
    )
    expect(container.firstChild).toBeNull()
  })

  it('says a lane owns no work yet, and how work joins it', () => {
    render(<VersionsStrip lane={{ ...lane, epics: [] }} versions={[]} labels={labels} />)
    expect(screen.getByText(labels.empty)).toBeTruthy()
    expect(screen.getByText(labels.emptyHint)).toBeTruthy()
  })

  // The lane's number, not a version's: this is the whole point of the strip.
  it('reports the lane total across its versions', () => {
    render(
      <VersionsStrip
        lane={lane}
        versions={[
          epic({ id: 'demo-v1', title: 'Billing v1', total: 2, done: 2, percent: 100 }),
          epic({ id: 'demo-v2', title: 'Billing v2', total: 3, done: 1, percent: 33, ready: 2 }),
        ]}
        labels={labels}
      />,
    )
    expect(screen.getByText('3 / 5')).toBeTruthy()
    expect(screen.getByText(/done · 60%/)).toBeTruthy()
    expect(screen.getByText('Billing v1')).toBeTruthy()
    expect(screen.getByText('Billing v2')).toBeTruthy()
  })

  it('exposes each version’s progress as a readable number, not just a width', () => {
    render(
      <VersionsStrip
        lane={lane}
        versions={[epic({ id: 'demo-v2', title: 'Billing v2', percent: 33 })]}
        labels={labels}
      />,
    )
    const bar = screen.getByRole('progressbar')
    expect(bar.getAttribute('aria-valuenow')).toBe('33')
  })

  it('links a version to its ticket only when given a href builder', () => {
    const versions = [epic({ id: 'demo-v2', title: 'Billing v2' })]
    render(
      <VersionsStrip
        lane={lane}
        versions={versions}
        epicHref={(id) => `/board#t=${id}`}
        labels={labels}
      />,
    )
    expect(screen.getByRole('link', { name: 'Billing v2' }).getAttribute('href')).toBe(
      '/board#t=demo-v2',
    )
    cleanup()
    render(<VersionsStrip lane={lane} versions={versions} labels={labels} />)
    expect(screen.queryByRole('link')).toBeNull()
  })

  it('surfaces a parked lane the queue is still feeding', () => {
    render(
      <VersionsStrip
        lane={{ ...lane, status: 'parked' }}
        versions={[]}
        warnings={['parked_with_ready_work']}
        labels={labels}
      />,
    )
    expect(screen.getByText(labels.parkedWithReadyWork)).toBeTruthy()
  })

  // The state is `done` here so the state BADGE cannot be mistaken for the
  // count label of the same name.
  it('omits a version’s claimable counts when there are none', () => {
    render(
      <VersionsStrip
        lane={lane}
        versions={[
          epic({
            id: 'demo-v1',
            title: 'Billing v1',
            state: 'done',
            state_category: 'done',
            total: 2,
            done: 2,
            percent: 100,
          }),
        ]}
        labels={labels}
      />,
    )
    expect(screen.queryByText(/^ready /)).toBeNull()
    expect(screen.queryByText(/^backlog /)).toBeNull()
  })
})
