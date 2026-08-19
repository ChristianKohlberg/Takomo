// The minimap's geometry, which is the only part of it a test can see: jsdom has
// no layout engine, so a rendered component proves nothing about where a box
// landed (web/README.md).
//
// What is worth pinning down is the handful of shapes the roadmap can produce
// that a naive tree walk gets wrong — an epic in two lanes, an epic in none, a
// lane with no work — plus the one colour decision the map makes.
import { describe, it, expect } from 'vitest'
import {
  NODE_H,
  PADDING,
  UNFILED,
  isComplete,
  lanesOf,
  layoutMap,
} from './initiative-map'
import type { Roadmap, RoadmapEpic, RoadmapLane } from './roadmap'

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

function roadmap(epics: RoadmapEpic[], initiatives: RoadmapLane[]): Roadmap {
  return { project: 'demo', generated_at: '2026-01-01T00:00:00Z', epics, initiatives }
}

describe('lanesOf', () => {
  it('resolves each lane to its epics, in the lane order', () => {
    const rm = roadmap(
      [epic('t-2', { initiatives: ['ini-a'] }), epic('t-1', { initiatives: ['ini-a'] })],
      [lane('ini-a', ['t-1', 't-2'])],
    )
    const [first] = lanesOf(rm)
    expect(first!.epics.map((e) => e.id)).toEqual(['t-1', 't-2'])
  })

  // Lanes are grown from an `initiative:` tag and a ticket may carry two, so the
  // roadmap deliberately does not partition the project. Drawing the epic once
  // would silently drop a branch someone filed.
  it('draws an epic filed under two initiatives under both', () => {
    const rm = roadmap(
      [epic('t-1', { initiatives: ['ini-a', 'ini-b'] })],
      [lane('ini-a', ['t-1']), lane('ini-b', ['t-1'])],
    )
    const lanes = lanesOf(rm)
    expect(lanes.map((l) => l.epics.map((e) => e.id))).toEqual([['t-1'], ['t-1']])

    // …and the two boxes stay distinct, which is what the lane-qualified key is for.
    const keys = layoutMap('demo', lanes)
      .nodes.filter((n) => n.kind === 'epic')
      .map((n) => n.key)
    expect(new Set(keys).size).toBe(2)
  })

  // `uninitiated` exists so percentages cannot read as complete while real work
  // sits outside every lane; a map that drew only filed epics would bring the
  // problem straight back.
  it('collects epics under no initiative into a trailing unfiled lane', () => {
    const rm = roadmap(
      [
        epic('t-1', { initiatives: ['ini-a'], total: 4, done: 4 }),
        epic('t-9', { total: 3, done: 1 }),
        epic('t-8', { total: 1, done: 0 }),
      ],
      [lane('ini-a', ['t-1'])],
    )
    const lanes = lanesOf(rm)
    expect(lanes).toHaveLength(2)
    expect(lanes[1]!.unfiled).toBe(true)
    expect(lanes[1]!.lane.id).toBe(UNFILED)
    expect(lanes[1]!.epics.map((e) => e.id)).toEqual(['t-9', 't-8'])
    // Summed from the epics drawn beside it, so the numbers on screen add up.
    expect(lanes[1]!.lane.total).toBe(4)
    expect(lanes[1]!.lane.done).toBe(1)
    expect(lanes[1]!.lane.percent).toBe(25)
  })

  it('adds no unfiled lane when every epic is filed', () => {
    const rm = roadmap([epic('t-1', { initiatives: ['ini-a'] })], [lane('ini-a', ['t-1'])])
    expect(lanesOf(rm).some((l) => l.unfiled)).toBe(false)
  })

  // A stale response naming a version that is being deleted should lose a row,
  // not gain an untitled one.
  it('drops an id that names no epic', () => {
    const rm = roadmap([epic('t-1', { initiatives: ['ini-a'] })], [lane('ini-a', ['t-1', 't-gone'])])
    expect(lanesOf(rm)[0]!.epics.map((e) => e.id)).toEqual(['t-1'])
  })

  it('is empty without a roadmap', () => {
    expect(lanesOf(undefined)).toEqual([])
  })
})

describe('layoutMap', () => {
  it('puts the project, its initiatives and their epics in three columns', () => {
    const rm = roadmap([epic('t-1', { initiatives: ['ini-a'] })], [lane('ini-a', ['t-1'])])
    const { nodes } = layoutMap('demo', lanesOf(rm))
    const x = (kind: string) => nodes.find((n) => n.kind === kind)!.x
    expect(x('root')).toBeLessThan(x('lane'))
    expect(x('lane')).toBeLessThan(x('epic'))
  })

  it('centres a lane against the block of its epics', () => {
    const rm = roadmap(
      ['t-1', 't-2', 't-3'].map((id) => epic(id, { initiatives: ['ini-a'] })),
      [lane('ini-a', ['t-1', 't-2', 't-3'])],
    )
    const { nodes } = layoutMap('demo', lanesOf(rm))
    const epics = nodes.filter((n) => n.kind === 'epic')
    const laneNode = nodes.find((n) => n.kind === 'lane')!
    const top = Math.min(...epics.map((n) => n.y))
    const bottom = Math.max(...epics.map((n) => n.y + n.height))
    expect(laneNode.y + laneNode.height / 2).toBeCloseTo((top + bottom) / 2, 5)
  })

  // An initiative opened before any work exists is the normal case, not an edge
  // one: it has to render as itself rather than collapse to nothing.
  it('gives a lane with no epics a row of its own', () => {
    const rm = roadmap([], [lane('ini-a', []), lane('ini-b', [])])
    const { nodes } = layoutMap('demo', lanesOf(rm))
    const lanesOut = nodes.filter((n) => n.kind === 'lane')
    expect(lanesOut).toHaveLength(2)
    expect(lanesOut[0]!.y).not.toBe(lanesOut[1]!.y)
  })

  it('never places a node above the padding', () => {
    const rm = roadmap(
      ['t-1', 't-2', 't-3', 't-4'].map((id) => epic(id, { initiatives: ['ini-a'] })),
      [lane('ini-a', ['t-1', 't-2', 't-3', 't-4'])],
    )
    const { nodes, height } = layoutMap('demo', lanesOf(rm))
    expect(Math.min(...nodes.map((n) => n.y))).toBeGreaterThanOrEqual(PADDING)
    expect(height).toBeGreaterThanOrEqual(Math.max(...nodes.map((n) => n.y + n.height)))
  })

  it('links the root to every lane and each lane to its own epics', () => {
    const rm = roadmap(
      [epic('t-1', { initiatives: ['ini-a'] }), epic('t-2', { initiatives: ['ini-b'] })],
      [lane('ini-a', ['t-1']), lane('ini-b', ['t-2'])],
    )
    const { edges } = layoutMap('demo', lanesOf(rm))
    expect(edges.filter((e) => e.from === 'root').map((e) => e.to)).toEqual([
      'l:ini-a',
      'l:ini-b',
    ])
    expect(edges).toContainEqual({ from: 'l:ini-a', to: 'e:ini-a:t-1' })
    expect(edges).toContainEqual({ from: 'l:ini-b', to: 'e:ini-b:t-2' })
  })

  // The property the centring arithmetic is most likely to break, and the one a
  // reader notices instantly: two boxes drawn on top of each other. Checked
  // within a column, since columns cannot collide by construction.
  it('never overlaps two boxes in the same column', () => {
    const rm = roadmap(
      [
        ...['a1', 'a2', 'a3', 'a4', 'a5'].map((id) => epic(id, { initiatives: ['ini-a'] })),
        epic('b1', { initiatives: ['ini-b'] }),
        ...['c1', 'c2'].map((id) => epic(id, { initiatives: ['ini-c'] })),
        epic('x1'),
      ],
      [
        lane('ini-a', ['a1', 'a2', 'a3', 'a4', 'a5']),
        lane('ini-b', ['b1']),
        lane('ini-empty', []),
        lane('ini-c', ['c1', 'c2']),
      ],
    )
    const { nodes, width, height } = layoutMap('demo', lanesOf(rm))
    const columns = new Map<number, typeof nodes>()
    for (const n of nodes) {
      const col = columns.get(n.x) ?? []
      col.push(n)
      columns.set(n.x, col)
    }
    for (const col of columns.values()) {
      const sorted = [...col].sort((a, b) => a.y - b.y)
      for (let i = 1; i < sorted.length; i++) {
        expect(sorted[i]!.y).toBeGreaterThanOrEqual(sorted[i - 1]!.y + sorted[i - 1]!.height)
      }
    }
    // …and the canvas the component sizes actually contains all of it.
    for (const n of nodes) {
      expect(n.x + n.width).toBeLessThanOrEqual(width)
      expect(n.y + n.height).toBeLessThanOrEqual(height)
    }
  })

  it('draws a root on its own when the project has nothing yet', () => {
    const { nodes, height } = layoutMap('demo', [])
    expect(nodes).toHaveLength(1)
    expect(nodes[0]!.kind).toBe('root')
    expect(height).toBe(PADDING * 2 + NODE_H)
  })
})

describe('isComplete', () => {
  it('is true only for an epic whose work is all done', () => {
    expect(isComplete(epic('t-1', { total: 4, done: 4 }))).toBe(true)
    expect(isComplete(epic('t-2', { total: 4, done: 3 }))).toBe(false)
  })

  // 0/0 is not an achievement. An epic filed ahead of its work is legitimate —
  // the roadmap flags it rather than erroring — but painting it green would say
  // it shipped.
  it('is false for an epic with no work under it', () => {
    expect(isComplete(epic('t-3', { total: 0, done: 0, percent: 0 }))).toBe(false)
  })
})
