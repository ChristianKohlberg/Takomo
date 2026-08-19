// What the map does that a reader depends on, minus the geometry — where a box
// landed is `lib/initiative-map.test.ts`'s job, because jsdom has no layout
// engine (web/README.md).
//
// What is left is worth testing precisely because it is the part that carries
// meaning: which boxes exist, which one is green, and what a click on each kind
// of node is supposed to produce.
import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, cleanup, fireEvent, within } from '@testing-library/react'
import { Minimap } from './Minimap'
import { lanesOf } from '@/lib/initiative-map'
import type { Roadmap, RoadmapEpic, RoadmapLane } from '@/lib/roadmap'

afterEach(cleanup)

const labels = {
  heading: 'Project map',
  needProject: 'Pick a project to see its map',
  needProjectHint: 'The map draws one project at a time.',
  empty: 'Nothing to draw yet',
  emptyHint: 'Create an initiative.',
  unfiled: 'No initiative',
  unfiledHint: 'Epics no initiative claims.',
  goTo: 'Go to',
  openDocument: 'Open document',
  openVerification: 'Verification',
  versions: 'Versions',
  statistics: 'Statistics',
  close: 'Close',
  done: 'done',
  ready: 'Ready',
  backlog: 'Backlog',
  awaiting: 'Awaiting an answer',
  state: 'By state',
  priority: 'Priority',
  openOnBoard: 'Open on the board',
  noWork: 'No work is filed under this epic yet.',
  zoomIn: 'Zoom in',
  zoomOut: 'Zoom out',
  complete: 'Complete',
  pickEpic: 'Pick an epic on the map to see its numbers.',
}

function epic(id: string, over: Partial<RoadmapEpic> = {}): RoadmapEpic {
  return {
    id,
    title: id,
    state: 'open',
    state_category: 'open',
    priority: 'normal',
    total: 0,
    done: 0,
    percent: 0,
    ready: 0,
    backlog: 0,
    awaiting_answer: 0,
    flags: [],
    initiatives: [],
    ...over,
  }
}

function lane(id: string, epics: string[], over: Partial<RoadmapLane> = {}): RoadmapLane {
  return {
    id,
    title: id,
    status: 'open',
    total: 0,
    done: 0,
    percent: 0,
    ready: 0,
    backlog: 0,
    awaiting_answer: 0,
    epics,
    flags: [],
    ...over,
  }
}

function show(rm: Roadmap | undefined, over: Partial<Parameters<typeof Minimap>[0]> = {}) {
  const onOpenInitiative = vi.fn()
  render(
    <Minimap
      rootLabel="demo"
      lanes={lanesOf(rm)}
      hasProject
      onOpenInitiative={onOpenInitiative}
      epicHref={(id) => `/board#t=${id}`}
      verificationHref="/verification"
      labels={labels}
      {...over}
    />,
  )
  return { onOpenInitiative }
}

describe('Minimap', () => {
  it('asks for a project rather than drawing an empty map for all of them', () => {
    show(undefined, { hasProject: false })
    expect(screen.getByText(labels.needProject)).toBeTruthy()
    expect(screen.queryByText(labels.heading)).toBeNull()
  })

  it('draws the project, its initiatives and their epics', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [
        epic('t-1', { title: 'v1', initiatives: ['ini-a'] }),
        epic('t-2', { title: 'v2', initiatives: ['ini-a'] }),
      ],
      initiatives: [lane('ini-a', ['t-1', 't-2'], { title: 'Billing' })],
    })
    expect(screen.getByText('demo')).toBeTruthy()
    expect(screen.getByText('Billing')).toBeTruthy()
    expect(screen.getByText('v1')).toBeTruthy()
    expect(screen.getByText('v2')).toBeTruthy()
  })

  // The one colour decision. A finished epic is green; nothing else is, so green
  // reads as "nothing left here" rather than as a category.
  it('greens a finished epic and leaves every other one neutral', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [
        epic('t-1', { title: 'shipped', total: 4, done: 4, percent: 100, initiatives: ['ini-a'] }),
        epic('t-2', { title: 'nearly', total: 4, done: 3, percent: 75, initiatives: ['ini-a'] }),
        epic('t-3', { title: 'empty', initiatives: ['ini-a'] }),
      ],
      initiatives: [lane('ini-a', ['t-1', 't-2', 't-3'])],
    })
    const box = (title: string) => screen.getByText(title).closest('button')!
    expect(box('shipped').className).toContain('bg-okbg')
    expect(box('nearly').className).not.toContain('bg-okbg')
    // 0/0 is not an achievement: an epic filed ahead of its work is legitimate,
    // and painting it green would say it shipped.
    expect(box('empty').className).not.toContain('bg-okbg')
  })

  it('shows an epic its numbers when it is clicked', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [
        epic('t-1', {
          title: 'v1',
          total: 10,
          done: 4,
          percent: 40,
          ready: 2,
          backlog: 3,
          awaiting_answer: 1,
          by_state: { open: 3, in_progress: 2, done: 4, blocked: 1 },
          initiatives: ['ini-a'],
        }),
      ],
      initiatives: [lane('ini-a', ['t-1'])],
    })
    expect(screen.getByText(labels.pickEpic)).toBeTruthy()

    fireEvent.click(screen.getByText('v1'))
    expect(screen.getByText(labels.statistics)).toBeTruthy()
    expect(screen.getByText('40%')).toBeTruthy()
    expect(screen.getByText(/4 \/ 10/)).toBeTruthy()
    // by_state, because "eleven left" and "eleven left, nine blocked" are
    // different situations the totals cannot tell apart.
    expect(screen.getByText('blocked')).toBeTruthy()
    expect(screen.getByText(labels.awaiting)).toBeTruthy()
    expect(screen.getByText(/Open on the board/).getAttribute('href')).toBe('/board#t=t-1')
  })

  it('says so rather than showing a row of zeroes for an epic with no work', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [epic('t-1', { title: 'planned', initiatives: ['ini-a'] })],
      initiatives: [lane('ini-a', ['t-1'])],
    })
    fireEvent.click(screen.getByText('planned'))
    expect(screen.getByText(labels.noWork)).toBeTruthy()
    expect(screen.queryByText(labels.state)).toBeNull()
  })

  it('opens a go-to menu on an initiative, listing its versions', () => {
    const { onOpenInitiative } = show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [epic('t-1', { title: 'v1', initiatives: ['ini-a'] })],
      initiatives: [lane('ini-a', ['t-1'], { title: 'Billing' })],
    })
    expect(screen.queryByRole('menu')).toBeNull()

    fireEvent.click(screen.getByText('Billing'))
    const menu = screen.getByRole('menu', { name: labels.goTo })
    expect(within(menu).getByText(labels.openDocument)).toBeTruthy()
    expect(within(menu).getByText(labels.openVerification).getAttribute('href')).toBe(
      '/verification',
    )
    expect(within(menu).getByText('t-1').getAttribute('href')).toBe('/board#t=t-1')

    fireEvent.click(within(menu).getByText(labels.openDocument))
    expect(onOpenInitiative).toHaveBeenCalledWith('ini-a')
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('closes the go-to menu on Escape', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [],
      initiatives: [lane('ini-a', [], { title: 'Billing' })],
    })
    fireEvent.click(screen.getByText('Billing'))
    expect(screen.getByRole('menu')).toBeTruthy()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
  })

  // The unfiled bucket is a statement about the work, not an initiative anybody
  // created, so there is no document behind it to offer.
  it('offers no document for the unfiled bucket', () => {
    show({
      project: 'demo',
      generated_at: '2026-01-01T00:00:00Z',
      epics: [epic('t-9', { title: 'stray' })],
      initiatives: [],
    })
    fireEvent.click(screen.getByText(labels.unfiled))
    const menu = screen.getByRole('menu')
    expect(within(menu).queryByText(labels.openDocument)).toBeNull()
    expect(within(menu).getByText('t-9').getAttribute('href')).toBe('/board#t=t-9')
  })
})
