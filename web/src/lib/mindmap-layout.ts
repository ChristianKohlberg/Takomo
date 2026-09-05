// The geometry of a mindmap canvas: layout, viewport, hit testing, drops.
//
// Every decision the canvas makes lives here as a pure function, and the
// component is a renderer plus pointer handlers that call them. That split is not
// tidiness — jsdom has no layout engine, so nothing that reads real element
// geometry can be tested at all. Keeping the *decisions* out of the DOM is what
// makes them provable, and leaves the untestable part down to "did it draw".
//
// Two coordinate spaces, and confusing them is the bug this module exists to
// prevent:
//
//   world  — where nodes live. Stable, pans and zooms under the viewport.
//   screen — pixels in the SVG element. What a pointer event gives you.
//
// `toWorld`/`toScreen` are the only crossings, and both are tested against each
// other.

/** A point in either space; which one is always in the parameter name. */
export interface Point {
  x: number
  y: number
}

/**
 * The only thing this module needs to know about a node.
 *
 * Deliberately structural rather than the REST or CRDT node type: geometry cares
 * about identity, parentage, sibling rank and hand placement, and nothing else.
 * Both `MindmapNode` (the REST read) and `MapNode` (the shared document) satisfy
 * it, so the same tested geometry serves both — and `position` stays an integer
 * rank here even though the document orders siblings with a fractional index
 * string, because ranking is what the layout actually consumes.
 */
export interface LayoutNode {
  id: string
  /** null = a first-ring branch off the root. */
  parent: string | null
  /** Sibling rank, ascending. Ties broken by id so the order is total. */
  position: number
  /** Hand placement, or null to let the layout place it. */
  at: Point | null
}

/** Where the canvas is looking: a pan offset in screen pixels, and a zoom. */
export interface Viewport {
  x: number
  y: number
  zoom: number
}

export const DEFAULT_VIEWPORT: Viewport = { x: 0, y: 0, zoom: 1 }

/** Zoom bounds. Past these a mind map is either unreadable or a single word. */
export const MIN_ZOOM = 0.25
export const MAX_ZOOM = 2.5

/** Node box, in world units. Fixed size: a mindmap node is a sentence or two. */
/**
 * How far past a node's right edge its hover affordances reach.
 *
 * The `+` that adds a child is drawn just outside the box, where the child will
 * appear. Hover has to reach it: tested against the box alone, moving the
 * pointer toward the button leaves the node, un-hovers it, and unmounts the
 * button before the click can land — so the affordance is only usable on a node
 * that happens to be selected. That is a bug you can only find with a mouse.
 */
export const AFFORDANCE_WIDTH = 30

export const NODE_WIDTH = 190
export const NODE_HEIGHT = 52
/** Gap between a parent's column and its children's. */
export const COLUMN_GAP = 78
/** Gap between two stacked siblings. */
export const ROW_GAP = 16

/** A node placed in world space, ready to draw. */
export interface PlacedNode<T extends LayoutNode = LayoutNode> {
  node: T
  /** Top-left corner, world units. */
  x: number
  y: number
  depth: number
  /** Whether this position came from the node's own `at` rather than the layout. */
  pinned: boolean
}

export interface Layout<T extends LayoutNode = LayoutNode> {
  nodes: PlacedNode<T>[]
  /** The root box — the map's title, which is not a node. */
  root: { x: number; y: number }
  /** World-space bounds of everything, for "fit to view". */
  bounds: { minX: number; minY: number; maxX: number; maxY: number }
}

/** Children of each node, in position order. `null` keys the first ring. */
export function childrenOf<T extends LayoutNode>(nodes: readonly T[]): Map<string | null, T[]> {
  const out = new Map<string | null, T[]>()
  for (const node of nodes) {
    const key = node.parent ?? null
    const list = out.get(key)
    if (list) list.push(node)
    else out.set(key, [node])
  }
  for (const list of out.values()) {
    list.sort((a, b) => a.position - b.position || a.id.localeCompare(b.id))
  }
  return out
}

/**
 * Lay the tree out left-to-right: the root on the left, each ring a column, and
 * every subtree given exactly the vertical room it needs.
 *
 * A tidy-tree rather than a radial burst, for a reason that only shows up while
 * somebody is typing fast: in a radial layout adding one node re-angles its whole
 * ring, so the map jumps under the cursor. Here a new node extends its own column
 * downward and everything above it stays put.
 *
 * A node with `at` set is honoured exactly and **taken out of the flow** — its
 * siblings close up as if it were not there, because a pinned node is somewhere
 * the person put it on purpose and should not push the rest around.
 */
export function layout<T extends LayoutNode>(nodes: readonly T[]): Layout<T> {
  const kids = childrenOf(nodes)
  const placed: PlacedNode<T>[] = []
  // A running vertical cursor: every leaf takes the next row, and a parent is
  // centred against the rows its subtree took. One pass, no second
  // resolve-overlaps phase.
  let cursor = 0

  const walk = (parentId: string | null, depth: number): void => {
    for (const node of kids.get(parentId) ?? []) {
      if (node.at != null) {
        // Placed by hand: honoured exactly, and taken out of the flow so its
        // siblings close up as if it were not there. A pinned node is somewhere
        // somebody put it on purpose and should not push the rest around.
        placed.push({ node, x: node.at.x, y: node.at.y, depth, pinned: true })
        // Its children still hang off where it actually is, or dragging a node
        // would leave its subtree behind.
        walk(node.id, depth + 1)
        continue
      }
      const before = cursor
      walk(node.id, depth + 1)
      const y =
        cursor === before
          ? // A leaf takes the next row.
            (() => {
              const row = cursor
              cursor += NODE_HEIGHT + ROW_GAP
              return row
            })()
          : // A parent sits level with the middle of its children, which is what
            // makes a tree readable at a glance.
            centreAgainstChildren(node.id, placed, before)
      placed.push({
        node,
        x: depth * (NODE_WIDTH + COLUMN_GAP),
        y,
        depth,
        pinned: false,
      })
    }
  }

  walk(null, 1)

  // The root sits level with the middle of the first ring, in its own column to
  // the left of it.
  const firstRing = placed.filter((p) => p.depth === 1 && !p.pinned)
  const rootY =
    firstRing.length > 0
      ? (Math.min(...firstRing.map((p) => p.y)) + Math.max(...firstRing.map((p) => p.y))) / 2
      : 0

  const rootX = -(NODE_WIDTH + COLUMN_GAP)
  const xs = placed.map((p) => p.x).concat(rootX)
  const ys = placed.map((p) => p.y).concat(rootY)
  return {
    nodes: placed,
    root: { x: rootX, y: rootY },
    bounds: {
      minX: Math.min(...xs),
      minY: Math.min(...ys),
      maxX: Math.max(...xs) + NODE_WIDTH,
      maxY: Math.max(...ys) + NODE_HEIGHT,
    },
  }
}

/**
 * Centre a parent against the children already placed beneath it.
 *
 * Called after the subtree is laid out, so those children are in `placed` and
 * their extent is known. A parent whose children were all dragged away has
 * nothing to centre against and keeps the row the flow gave it.
 */
function centreAgainstChildren(
  id: string,
  placed: readonly PlacedNode<LayoutNode>[],
  fallbackTop: number,
): number {
  const mine = placed.filter((p) => p.node.parent === id && !p.pinned)
  if (mine.length === 0) return fallbackTop
  const top = Math.min(...mine.map((p) => p.y))
  const bottom = Math.max(...mine.map((p) => p.y + NODE_HEIGHT))
  return (top + bottom) / 2 - NODE_HEIGHT / 2
}

/** Screen pixels → world units. */
export function toWorld(screen: Point, viewport: Viewport): Point {
  return {
    x: (screen.x - viewport.x) / viewport.zoom,
    y: (screen.y - viewport.y) / viewport.zoom,
  }
}

/** World units → screen pixels. */
export function toScreen(world: Point, viewport: Viewport): Point {
  return {
    x: world.x * viewport.zoom + viewport.x,
    y: world.y * viewport.zoom + viewport.y,
  }
}

/**
 * Zoom about a fixed screen point — the cursor, so the thing under it stays under
 * it. Zooming about the origin instead is the classic canvas bug: the map slides
 * away from wherever you were looking.
 */
export function zoomAt(viewport: Viewport, screen: Point, factor: number): Viewport {
  const zoom = clamp(viewport.zoom * factor, MIN_ZOOM, MAX_ZOOM)
  // The world point under the cursor must not move, so solve for the pan that
  // keeps it: screen = world * zoom + pan.
  const world = toWorld(screen, viewport)
  return {
    zoom,
    x: screen.x - world.x * zoom,
    y: screen.y - world.y * zoom,
  }
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

/** The topmost placed node containing a world point, or null. */
export function nodeAt<T extends LayoutNode>(
  placed: readonly PlacedNode<T>[],
  world: Point,
  padRight = 0,
): PlacedNode<T> | null {
  // Reverse order so a node drawn on top wins, matching what the eye expects.
  for (let i = placed.length - 1; i >= 0; i -= 1) {
    const p = placed[i]!
    if (
      world.x >= p.x &&
      world.x <= p.x + NODE_WIDTH + padRight &&
      world.y >= p.y &&
      world.y <= p.y + NODE_HEIGHT
    ) {
      return p
    }
  }
  return null
}

/** What releasing a drag over `world` means. */
export type Drop =
  | { kind: 'reparent'; parent: string }
  | { kind: 'place'; at: Point }
  | { kind: 'none' }

/**
 * Resolve a drop: onto another node reparents, into space pins the position.
 *
 * The refusals are here rather than left to the server because a drag that only
 * fails on release is a bad gesture — the canvas can grey the target instead.
 * Dropping onto the node itself or onto its own descendant is refused: the second
 * would cut that branch off the map, and it is exactly what a drag makes easy to
 * try.
 */
export function resolveDrop<T extends LayoutNode>(
  placed: readonly PlacedNode<T>[],
  dragged: LayoutNode,
  world: Point,
  topLeft: Point,
): Drop {
  const over = nodeAt(placed, world)
  if (over && over.node.id !== dragged.id) {
    if (isDescendant(placed, over.node.id, dragged.id)) return { kind: 'none' }
    return { kind: 'reparent', parent: over.node.id }
  }
  if (over && over.node.id === dragged.id) return { kind: 'none' }
  return { kind: 'place', at: topLeft }
}

/** Is `candidate` inside `ancestor`'s subtree? */
export function isDescendant(
  placed: readonly PlacedNode<LayoutNode>[],
  candidate: string,
  ancestor: string,
): boolean {
  const parentOf = new Map(placed.map((p) => [p.node.id, p.node.parent ?? null]))
  let cursor: string | null = candidate
  // Bounded by the node count: a cycle in hand-edited data must not spin here.
  for (let i = 0; i <= placed.length; i += 1) {
    if (cursor === null) return false
    if (cursor === ancestor) return true
    cursor = parentOf.get(cursor) ?? null
  }
  return false
}

/**
 * Which side of each box an edge leaves and arrives on.
 *
 * `right` is the tidy tree's fixed answer — parent's right edge to child's left —
 * and stays the default so nothing that called this with two arguments changes.
 * `auto` is what a radial ring needs: half its branches grow leftward, and an
 * edge that insists on leaving the right edge there loops back across its own
 * node.
 */
export type EdgeDirection = 'right' | 'left' | 'auto'

/** The curve between two node boxes, leaving and arriving level. */
export function edgePath(from: Point, to: Point, direction: EdgeDirection = 'right'): string {
  const leftward = direction === 'left' || (direction === 'auto' && to.x < from.x)
  const x1 = leftward ? from.x : from.x + NODE_WIDTH
  const y1 = from.y + NODE_HEIGHT / 2
  const x2 = leftward ? to.x + NODE_WIDTH : to.x
  const y2 = to.y + NODE_HEIGHT / 2
  // A horizontal-tangent cubic: it leaves and arrives level, so a branch reads as
  // one line rather than a diagonal that has to be traced.
  const mid = (x1 + x2) / 2
  return `M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`
}

// ---- radial ---------------------------------------------------------------
//
// `layout` above argues against a radial burst, and that argument is right about
// the thing it names: dividing the circle by the sibling count re-angles the
// whole ring every time somebody presses Enter, so the map jumps under the
// cursor mid-sentence. What follows keeps the shape and drops the jumping, by
// never dividing the circle at all.
//
// A first-ring node's angle is a function of its INDEX and nothing else — the
// base-2 radical inverse, the van der Corput sequence. Index 0 sits at 0, index 1
// at half a turn, index 2 at a quarter, index 3 at three quarters, and so on:
// every prefix of the sequence is spread evenly around the circle, and appending
// index n never moves indices 0…n-1. That is "a slot count that only grows"
// expressed as a formula rather than as a divisor that has to be recomputed.
//
// Only the first ring is radial. Everything below it is the same tidy sub-tree
// this module already draws, pointed outward from its branch — because the
// tidy-tree argument holds inside a branch, where the typing actually happens.

/** Radius of the smallest ring. Below this the boxes touch the root. */
export const RING_MIN_RADIUS = 260

/** The base-2 radical inverse: 0, ½, ¼, ¾, ⅛… — a permanent address for an index. */
function radicalInverse2(index: number): number {
  let result = 0
  let denominator = 1
  let n = Math.max(0, Math.floor(index))
  while (n > 0) {
    denominator *= 2
    result += (n % 2) / denominator
    n = Math.floor(n / 2)
  }
  return result
}

/**
 * The angle, in radians, of the first-ring node at `index`.
 *
 * Depends on the index alone, which is the whole property: appending a sibling
 * cannot move the ones already on the circle.
 */
export function ringAngle(index: number): number {
  return 2 * Math.PI * radicalInverse2(index)
}

/** Slots grow by doubling, so the radius changes rarely rather than continuously. */
function slotCount(count: number): number {
  let slots = 8
  while (slots < count) slots *= 2
  return slots
}

/**
 * How far out the first ring sits for a map of `count` branches.
 *
 * Stepped by the same doubling, so adding a branch inside the current step does
 * not slide the whole ring in or out either.
 */
export function ringRadius(count: number): number {
  return Math.max(RING_MIN_RADIUS, (slotCount(count) * (NODE_HEIGHT + ROW_GAP * 2)) / (2 * Math.PI))
}

/**
 * The root in the middle, its branches on a circle, each branch's own subtree
 * laid out tidily and pointed away from the centre.
 *
 * A node with `at` set is honoured exactly, exactly as in `layout` — but it still
 * consumes its ring index, so pinning one branch does not re-angle the others.
 */
export function radialLayout<T extends LayoutNode>(nodes: readonly T[]): Layout<T> {
  const kids = childrenOf(nodes)
  const placed: PlacedNode<T>[] = []
  const ring = kids.get(null) ?? []
  const radius = ringRadius(ring.length)

  ring.forEach((branch, index) => {
    const angle = ringAngle(index)
    // Top-left of the branch box: the ring places its CENTRE, so half a box comes
    // back off each axis.
    const home = branch.at ?? {
      x: Math.cos(angle) * radius - NODE_WIDTH / 2,
      y: Math.sin(angle) * radius - NODE_HEIGHT / 2,
    }
    placed.push({ node: branch, x: home.x, y: home.y, depth: 1, pinned: branch.at != null })

    // Grow away from the centre: a branch on the left half runs left, so its
    // subtree never crosses back over the root.
    const outward = (branch.at ? home.x + NODE_WIDTH / 2 : Math.cos(angle)) >= 0 ? 1 : -1
    const start = placed.length
    let cursor = 0

    const walk = (parentId: string, depth: number): void => {
      for (const node of kids.get(parentId) ?? []) {
        if (node.at != null) {
          placed.push({ node, x: node.at.x, y: node.at.y, depth: depth + 1, pinned: true })
          walk(node.id, depth + 1)
          continue
        }
        const before = cursor
        walk(node.id, depth + 1)
        const y =
          cursor === before
            ? (() => {
                const row = cursor
                cursor += NODE_HEIGHT + ROW_GAP
                return row
              })()
            : centreAgainstChildren(node.id, placed, before)
        placed.push({
          node,
          x: outward * depth * (NODE_WIDTH + COLUMN_GAP),
          y,
          depth: depth + 1,
          pinned: false,
        })
      }
    }
    walk(branch.id, 1)

    // The subtree was laid out around its own origin; slide it so its middle sits
    // level with the branch it hangs off, then out to where that branch is.
    const mine = placed.slice(start).filter((p) => !p.pinned)
    const shift =
      mine.length === 0
        ? 0
        : NODE_HEIGHT / 2 -
          (Math.min(...mine.map((p) => p.y)) + Math.max(...mine.map((p) => p.y)) + NODE_HEIGHT) / 2
    for (const p of mine) {
      p.x += home.x
      p.y += home.y + shift
    }
  })

  const rootX = -NODE_WIDTH / 2
  const rootY = -NODE_HEIGHT / 2
  const xs = placed.map((p) => p.x).concat(rootX)
  const ys = placed.map((p) => p.y).concat(rootY)
  return {
    nodes: placed,
    root: { x: rootX, y: rootY },
    bounds: {
      minX: Math.min(...xs),
      minY: Math.min(...ys),
      maxX: Math.max(...xs) + NODE_WIDTH,
      maxY: Math.max(...ys) + NODE_HEIGHT,
    },
  }
}

/**
 * A viewport that fits `bounds` inside a `width × height` viewport, with padding.
 *
 * What "fit" and the first render both use, so a map opens showing all of itself
 * rather than at whatever the last person's zoom happened to be.
 */
export function fit(
  bounds: Layout['bounds'],
  width: number,
  height: number,
  padding = 48,
): Viewport {
  const w = Math.max(1, bounds.maxX - bounds.minX)
  const h = Math.max(1, bounds.maxY - bounds.minY)
  const zoom = clamp(
    Math.min((width - padding * 2) / w, (height - padding * 2) / h),
    MIN_ZOOM,
    // Never zoom PAST 1 to fill space: a two-node map blown up to fill a monitor
    // looks broken, and the reader learns nothing from the extra pixels.
    1,
  )
  return {
    zoom,
    x: width / 2 - ((bounds.minX + bounds.maxX) / 2) * zoom,
    y: height / 2 - ((bounds.minY + bounds.maxY) / 2) * zoom,
  }
}

/**
 * Where a new sibling goes: between `after` and the node following it, or at the
 * end of the ring.
 *
 * Positions are gapped by 1000 in the store, so the midpoint is free. When two
 * siblings are already adjacent (a ring inserted into many times), it returns
 * null and the caller appends instead — re-gapping a whole ring mid-keystroke
 * would be a worse trade than one out-of-place node.
 */
export function positionAfter(siblings: readonly LayoutNode[], after: string): number | null {
  const index = siblings.findIndex((s) => s.id === after)
  if (index === -1) return null
  const current = siblings[index]!.position
  const next = siblings[index + 1]?.position
  if (next === undefined) return current + 1000
  if (next - current <= 1) return null
  return Math.floor((current + next) / 2)
}

/**
 * The viewport that puts one world point in the middle of the screen, at the
 * zoom the reader is already using.
 *
 * "Go to node…" is the only way to reach a node that is off screen now that the
 * rail is gone, so it must not also change the scale somebody chose — jumping to
 * a node AND rezooming loses the reader twice.
 */
export function centreOn(
  viewport: Viewport,
  world: Point,
  width: number,
  height: number,
): Viewport {
  return {
    zoom: viewport.zoom,
    x: width / 2 - world.x * viewport.zoom,
    y: height / 2 - world.y * viewport.zoom,
  }
}
