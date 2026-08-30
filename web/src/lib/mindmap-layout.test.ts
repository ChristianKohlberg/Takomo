// The canvas geometry, which is the half of a canvas that can be proven.
//
// jsdom has no layout engine, so a test can never show the map LOOKS right. What
// these show is every decision behind it: where a node lands, which node a click
// is on, what a drop means, and that the two coordinate spaces are actually
// inverses. A canvas whose maths is wrong looks plausible and behaves wrongly,
// which is the failure mode worth spending tests on.
import { describe, expect, it } from 'vitest'
import {
  centreOn,
  childrenOf,
  edgePath,
  radialLayout,
  ringAngle,
  ringRadius,
  fit,
  isDescendant,
  layout,
  nodeAt,
  positionAfter,
  resolveDrop,
  toScreen,
  toWorld,
  zoomAt,
  MAX_ZOOM,
  MIN_ZOOM,
  NODE_HEIGHT,
  NODE_WIDTH,
} from './mindmap-layout'
import type { MindmapNode } from './mindmaps'

function node(over: Partial<MindmapNode> & { id: string }): MindmapNode {
  return {
    mindmap: 'mm-1',
    parent: null,
    text: 'a thought',
    position: 1000,
    at: null,
    promoted: null,
    created_at: '2026-01-01T00:00:00Z',
    ...over,
  }
}

/** The shape the feature was designed around: a root fanning into four. */
const TREE: MindmapNode[] = [
  node({ id: 'api', text: 'API', position: 1000 }),
  node({ id: 'int', text: 'integrations', position: 2000 }),
  node({ id: 'ver', text: 'versioning?', parent: 'api', position: 1000 }),
  node({ id: 'idem', text: 'idempotent retries', parent: 'api', position: 2000 }),
]

describe('childrenOf', () => {
  it('groups by parent and orders siblings by position', () => {
    const kids = childrenOf([
      node({ id: 'b', parent: 'api', position: 2000 }),
      node({ id: 'a', parent: 'api', position: 1000 }),
    ])
    expect(kids.get('api')!.map((n) => n.id)).toEqual(['a', 'b'])
  })

  it('keys the first ring on null, not on undefined', () => {
    // The store sends `parent: null`; a Map keyed on `undefined` would silently
    // hold a second, empty ring and the first-level nodes would never be drawn.
    const kids = childrenOf([node({ id: 'api' })])
    expect(kids.get(null)!.map((n) => n.id)).toEqual(['api'])
  })

  it('is stable when two siblings share a position', () => {
    // Concurrent appends can collide on a position; the order must still be the
    // same on every render, or nodes swap places as you type.
    const kids = childrenOf([
      node({ id: 'b', parent: 'api', position: 1000 }),
      node({ id: 'a', parent: 'api', position: 1000 }),
    ])
    expect(kids.get('api')!.map((n) => n.id)).toEqual(['a', 'b'])
  })
})

describe('layout', () => {
  it('puts each ring in its own column, left to right', () => {
    const { nodes } = layout(TREE)
    const api = nodes.find((p) => p.node.id === 'api')!
    const ver = nodes.find((p) => p.node.id === 'ver')!
    expect(api.depth).toBe(1)
    expect(ver.depth).toBe(2)
    expect(ver.x).toBeGreaterThan(api.x)
    expect(ver.x - api.x).toBe(NODE_WIDTH + 78)
  })

  it('centres a parent against its children', () => {
    const { nodes } = layout(TREE)
    const api = nodes.find((p) => p.node.id === 'api')!
    const ver = nodes.find((p) => p.node.id === 'ver')!
    const idem = nodes.find((p) => p.node.id === 'idem')!
    expect(api.y).toBeCloseTo((ver.y + idem.y) / 2)
  })

  it('stacks siblings without overlapping them', () => {
    const { nodes } = layout(TREE)
    const ver = nodes.find((p) => p.node.id === 'ver')!
    const idem = nodes.find((p) => p.node.id === 'idem')!
    expect(idem.y - ver.y).toBeGreaterThanOrEqual(NODE_HEIGHT)
  })

  it('leaves everything above a new node exactly where it was', () => {
    // The property that matters while somebody types fast: adding a thought must
    // not shuffle the map under the cursor.
    const before = layout(TREE)
    const after = layout([...TREE, node({ id: 'new', parent: 'int', position: 1000 })])
    const y = (l: typeof before, id: string) => l.nodes.find((p) => p.node.id === id)!.y
    expect(y(after, 'api')).toBe(y(before, 'api'))
    expect(y(after, 'ver')).toBe(y(before, 'ver'))
  })

  it('honours a pinned node exactly and takes it out of the flow', () => {
    const pinned = TREE.map((n) =>
      n.id === 'ver' ? { ...n, at: { x: 900, y: -300 } } : n,
    )
    const { nodes } = layout(pinned)
    const ver = nodes.find((p) => p.node.id === 'ver')!
    expect([ver.x, ver.y, ver.pinned]).toEqual([900, -300, true])
    // Its sibling closes up into the row it would have taken: a node somebody
    // dragged away should not keep pushing the rest around.
    const idem = nodes.find((p) => p.node.id === 'idem')!
    const fresh = layout(TREE.filter((n) => n.id !== 'ver'))
    expect(idem.y).toBe(fresh.nodes.find((p) => p.node.id === 'idem')!.y)
  })

  it('keeps a pinned node’s children hanging off where it actually is', () => {
    // Otherwise dragging a node leaves its subtree behind, connected by edges
    // that cross the whole canvas.
    const pinned = [
      node({ id: 'api', at: { x: 500, y: 500 } }),
      node({ id: 'ver', parent: 'api' }),
    ]
    const { nodes } = layout(pinned)
    expect(nodes.find((p) => p.node.id === 'ver')).toBeTruthy()
    expect(nodes.find((p) => p.node.id === 'ver')!.depth).toBe(2)
  })

  it('places the root level with the middle of the first ring', () => {
    const { nodes, root } = layout(TREE)
    const first = nodes.filter((p) => p.depth === 1)
    expect(root.y).toBeCloseTo(
      (Math.min(...first.map((p) => p.y)) + Math.max(...first.map((p) => p.y))) / 2,
    )
    expect(root.x).toBeLessThan(0)
  })

  it('survives an empty map', () => {
    const { nodes, root, bounds } = layout([])
    expect(nodes).toEqual([])
    expect(root.y).toBe(0)
    expect(bounds.maxX).toBeGreaterThan(bounds.minX)
  })
})

describe('the two coordinate spaces', () => {
  it('round-trips a point through both', () => {
    const viewport = { x: 120, y: -40, zoom: 1.75 }
    const world = { x: 42.5, y: -13.25 }
    const back = toWorld(toScreen(world, viewport), viewport)
    expect(back.x).toBeCloseTo(world.x)
    expect(back.y).toBeCloseTo(world.y)
  })

  it('zooms about the cursor, so what is under it stays under it', () => {
    // The classic canvas bug is zooming about the origin: the map slides away
    // from whatever you were looking at.
    const viewport = { x: 0, y: 0, zoom: 1 }
    const cursor = { x: 300, y: 200 }
    const before = toWorld(cursor, viewport)
    const zoomed = zoomAt(viewport, cursor, 1.5)
    const after = toWorld(cursor, zoomed)
    expect(after.x).toBeCloseTo(before.x)
    expect(after.y).toBeCloseTo(before.y)
    expect(zoomed.zoom).toBeCloseTo(1.5)
  })

  it('clamps zoom at both ends', () => {
    const tiny = zoomAt({ x: 0, y: 0, zoom: MIN_ZOOM }, { x: 0, y: 0 }, 0.1)
    expect(tiny.zoom).toBe(MIN_ZOOM)
    const huge = zoomAt({ x: 0, y: 0, zoom: MAX_ZOOM }, { x: 0, y: 0 }, 10)
    expect(huge.zoom).toBe(MAX_ZOOM)
  })
})

describe('fit', () => {
  it('shows the whole map', () => {
    const { bounds } = layout(TREE)
    const viewport = fit(bounds, 800, 600)
    const topLeft = toScreen({ x: bounds.minX, y: bounds.minY }, viewport)
    const bottomRight = toScreen({ x: bounds.maxX, y: bounds.maxY }, viewport)
    expect(topLeft.x).toBeGreaterThanOrEqual(0)
    expect(topLeft.y).toBeGreaterThanOrEqual(0)
    expect(bottomRight.x).toBeLessThanOrEqual(800)
    expect(bottomRight.y).toBeLessThanOrEqual(600)
  })

  it('never zooms past 1 to fill space', () => {
    // A two-node map blown up to fill a monitor looks broken, and the extra
    // pixels teach the reader nothing.
    const { bounds } = layout([node({ id: 'only' })])
    expect(fit(bounds, 2000, 1500).zoom).toBeLessThanOrEqual(1)
  })
})

describe('hit testing', () => {
  it('finds the node under a point, and nothing in the gaps', () => {
    const { nodes } = layout(TREE)
    const api = nodes.find((p) => p.node.id === 'api')!
    expect(nodeAt(nodes, { x: api.x + 5, y: api.y + 5 })!.node.id).toBe('api')
    expect(nodeAt(nodes, { x: api.x - 40, y: api.y })).toBeNull()
  })

  it('counts the edges of a node as inside it', () => {
    const { nodes } = layout(TREE)
    const api = nodes.find((p) => p.node.id === 'api')!
    expect(nodeAt(nodes, { x: api.x, y: api.y })!.node.id).toBe('api')
    expect(
      nodeAt(nodes, { x: api.x + NODE_WIDTH, y: api.y + NODE_HEIGHT })!.node.id,
    ).toBe('api')
  })
})

describe('resolveDrop', () => {
  const placed = layout(TREE).nodes
  const at = (id: string) => {
    const p = placed.find((n) => n.node.id === id)!
    return { x: p.x + 5, y: p.y + 5 }
  }

  it('dropping on another node reparents', () => {
    const dragged = TREE.find((n) => n.id === 'int')!
    expect(resolveDrop(placed, dragged, at('api'), { x: 0, y: 0 })).toEqual({
      kind: 'reparent',
      parent: 'api',
    })
  })

  it('dropping into space pins the position', () => {
    const dragged = TREE.find((n) => n.id === 'int')!
    const drop = resolveDrop(placed, dragged, { x: -900, y: -900 }, { x: -910, y: -920 })
    expect(drop).toEqual({ kind: 'place', at: { x: -910, y: -920 } })
  })

  it('refuses a drop onto the node’s own descendant', () => {
    // It would cut that branch off the map — and a drag is exactly what makes it
    // easy to try. Refusing here lets the canvas grey the target rather than
    // failing only on release.
    const dragged = TREE.find((n) => n.id === 'api')!
    expect(resolveDrop(placed, dragged, at('ver'), { x: 0, y: 0 })).toEqual({ kind: 'none' })
  })

  it('refuses a drop onto itself', () => {
    const dragged = TREE.find((n) => n.id === 'api')!
    expect(resolveDrop(placed, dragged, at('api'), { x: 0, y: 0 })).toEqual({ kind: 'none' })
  })
})

describe('isDescendant', () => {
  const placed = layout(TREE).nodes

  it('walks up the tree', () => {
    expect(isDescendant(placed, 'ver', 'api')).toBe(true)
    expect(isDescendant(placed, 'api', 'ver')).toBe(false)
    expect(isDescendant(placed, 'int', 'api')).toBe(false)
  })

  it('terminates on a cycle rather than spinning', () => {
    // Hand-edited data can contain one, and a canvas that hangs is worse than a
    // canvas that draws something odd.
    const cyclic = layout([
      node({ id: 'a', parent: 'b' }),
      node({ id: 'b', parent: 'a' }),
    ]).nodes
    expect(isDescendant(cyclic, 'a', 'zzz')).toBe(false)
  })
})

describe('positionAfter', () => {
  const siblings = [
    node({ id: 'a', position: 1000 }),
    node({ id: 'b', position: 2000 }),
  ]

  it('lands between two siblings', () => {
    expect(positionAfter(siblings, 'a')).toBe(1500)
  })

  it('appends past the last one', () => {
    expect(positionAfter(siblings, 'b')).toBe(3000)
  })

  it('gives up when there is no gap left, rather than reordering the ring', () => {
    // Re-gapping a whole ring mid-keystroke is a worse trade than one node
    // landing at the end; the caller appends instead.
    const tight = [node({ id: 'a', position: 1 }), node({ id: 'b', position: 2 })]
    expect(positionAfter(tight, 'a')).toBeNull()
  })

  it('returns null for a node that is not there', () => {
    expect(positionAfter(siblings, 'ghost')).toBeNull()
  })
})

describe('edgePath', () => {
  it('leaves the parent’s right edge and arrives at the child’s left', () => {
    const path = edgePath({ x: 0, y: 0 }, { x: 300, y: 100 })
    expect(path.startsWith(`M ${NODE_WIDTH} ${NODE_HEIGHT / 2}`)).toBe(true)
    expect(path).toContain(`300 ${100 + NODE_HEIGHT / 2}`)
    expect(path).toContain('C')
  })
})

describe('radialLayout', () => {
  // The tidy tree's own header argues against a radial burst, and it is right
  // about the thing it names: dividing the circle by the sibling count re-angles
  // the whole ring on every insert, so the map jumps under the cursor. These are
  // the tests that the radial variant does not do that.

  const ring = (n: number) =>
    Array.from({ length: n }, (_, i) => node({ id: `b${i}`, position: (i + 1) * 1000 }))

  it('gives a first-ring index an angle that never depends on the count', () => {
    // The van der Corput sequence: every index has a permanent address on the
    // circle, and every prefix of it is spread evenly.
    expect(ringAngle(0)).toBeCloseTo(0)
    expect(ringAngle(1)).toBeCloseTo(Math.PI)
    expect(ringAngle(2)).toBeCloseTo(Math.PI / 2)
    expect(ringAngle(3)).toBeCloseTo((3 * Math.PI) / 2)
  })

  it('leaves every existing branch exactly where it was when one is appended', () => {
    // The property the tidy layout was chosen for, kept while going radial.
    const before = radialLayout(ring(3))
    const after = radialLayout(ring(4))
    for (const id of ['b0', 'b1', 'b2']) {
      const a = before.nodes.find((p) => p.node.id === id)!
      const b = after.nodes.find((p) => p.node.id === id)!
      expect([b.x, b.y]).toEqual([a.x, a.y])
    }
  })

  it('holds that property across a whole ring filling up', () => {
    let previous = radialLayout(ring(1))
    for (let n = 2; n <= 8; n += 1) {
      const next = radialLayout(ring(n))
      for (const placed of previous.nodes) {
        const moved = next.nodes.find((p) => p.node.id === placed.node.id)!
        expect([moved.x, moved.y]).toEqual([placed.x, placed.y])
      }
      previous = next
    }
  })

  it('steps the radius rather than sliding it, so growth is rare and visible', () => {
    expect(ringRadius(4)).toBe(ringRadius(8))
    expect(ringRadius(9)).toBeLessThanOrEqual(ringRadius(33))
  })

  it('puts the whole first ring on one circle around the root', () => {
    const { nodes, root } = radialLayout(ring(6))
    const radii = nodes
      .filter((p) => p.depth === 1)
      .map((p) => Math.hypot(p.x - root.x, p.y - root.y))
    for (const r of radii) expect(r).toBeCloseTo(radii[0]!)
  })

  it('keeps deeper rings as tidy sub-trees, growing away from the centre', () => {
    const nodes = [
      node({ id: 'right', position: 1000 }),
      node({ id: 'left', position: 2000 }),
      node({ id: 'r-kid', parent: 'right', position: 1000 }),
      node({ id: 'l-kid', parent: 'left', position: 1000 }),
    ]
    const placed = radialLayout(nodes)
    const at = (id: string) => placed.nodes.find((p) => p.node.id === id)!
    // Index 0 sits at angle 0 (to the right), index 1 half a turn away.
    expect(at('r-kid').x).toBeGreaterThan(at('right').x)
    expect(at('l-kid').x).toBeLessThan(at('left').x)
    expect(at('r-kid').depth).toBe(2)
  })

  it('centres a branch’s subtree against the branch', () => {
    const nodes = [
      node({ id: 'b', position: 1000 }),
      node({ id: 'k1', parent: 'b', position: 1000 }),
      node({ id: 'k2', parent: 'b', position: 2000 }),
    ]
    const placed = radialLayout(nodes)
    const at = (id: string) => placed.nodes.find((p) => p.node.id === id)!
    expect(at('b').y).toBeCloseTo((at('k1').y + at('k2').y) / 2)
  })

  it('honours a pinned branch without re-angling the others', () => {
    const plain = radialLayout(ring(4))
    const withPin = radialLayout(
      ring(4).map((n) => (n.id === 'b1' ? { ...n, at: { x: 900, y: -400 } } : n)),
    )
    const pinned = withPin.nodes.find((p) => p.node.id === 'b1')!
    expect([pinned.x, pinned.y, pinned.pinned]).toEqual([900, -400, true])
    for (const id of ['b0', 'b2', 'b3']) {
      const a = plain.nodes.find((p) => p.node.id === id)!
      const b = withPin.nodes.find((p) => p.node.id === id)!
      expect([b.x, b.y]).toEqual([a.x, a.y])
    }
  })

  it('survives an empty map', () => {
    const { nodes, bounds } = radialLayout([])
    expect(nodes).toEqual([])
    expect(bounds.maxX).toBeGreaterThan(bounds.minX)
  })
})

describe('edgePath direction', () => {
  it('still leaves the right edge by default, as the tidy tree needs', () => {
    expect(edgePath({ x: 0, y: 0 }, { x: 300, y: 0 })).toBe(
      edgePath({ x: 0, y: 0 }, { x: 300, y: 0 }, 'right'),
    )
  })

  it('turns around for a branch that grows leftward', () => {
    // A radial ring runs both ways; an edge that insists on leaving the right
    // edge loops back across its own node.
    const path = edgePath({ x: 0, y: 0 }, { x: -400, y: 0 }, 'auto')
    expect(path.startsWith(`M 0 ${NODE_HEIGHT / 2}`)).toBe(true)
    expect(path).toContain(`${-400 + NODE_WIDTH} ${NODE_HEIGHT / 2}`)
  })

  it('leaves a rightward branch alone in auto mode', () => {
    expect(edgePath({ x: 0, y: 0 }, { x: 400, y: 0 }, 'auto')).toBe(
      edgePath({ x: 0, y: 0 }, { x: 400, y: 0 }),
    )
  })
})

describe('centreOn', () => {
  it('puts the point in the middle of the viewport', () => {
    const v = centreOn({ x: 0, y: 0, zoom: 1 }, { x: 300, y: 200 }, 800, 600)
    expect(toScreen({ x: 300, y: 200 }, v)).toEqual({ x: 400, y: 300 })
  })

  it('keeps the zoom the reader chose', () => {
    const v = centreOn({ x: -50, y: 90, zoom: 0.5 }, { x: 300, y: 200 }, 800, 600)
    expect(v.zoom).toBe(0.5)
    expect(toScreen({ x: 300, y: 200 }, v)).toEqual({ x: 400, y: 300 })
  })
})
