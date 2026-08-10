// Where the nodes go when nobody has said.
//
// A stored layout wins; this is what a workflow looks like the FIRST time it is
// opened, and for every workflow nobody ever drags. That makes it the common
// case rather than the fallback, so it has to produce something readable rather
// than merely non-overlapping.
//
// Layered left-to-right by distance from `initial`, which is the axis a
// lifecycle actually runs along: a ticket starts at the initial state and moves
// right. Ordering within a column is by CATEGORY, so the terminal outcomes
// (`done`, `cancelled`) settle to the bottom together instead of interleaving
// with live work — the picture then matches how the states are talked about.
//
// Deterministic: same document in, same positions out, no randomness and no
// measurement of rendered text. That is what lets it be unit tested, and what
// stops a workflow from rearranging itself between two visits.
import type { WorkflowDoc } from '@/lib/workflows'
import type { Layout } from '@/lib/workflows'

export const NODE_W = 168
export const NODE_H = 56
export const COL_GAP = 96
export const ROW_GAP = 28
export const PAD = 24

/** Vertical ordering within a rank. Live work first, endings last. */
const CATEGORY_ORDER: Record<string, number> = {
  todo: 0,
  in_progress: 1,
  review: 2,
  blocked: 3,
  done: 4,
  cancelled: 5,
}

/**
 * Rank every state by its shortest distance from `initial`.
 *
 * States unreachable from `initial` still get a rank — the LAST one — rather
 * than being dropped. An unreachable state is a defect the validator reports,
 * but the editor is exactly where someone goes to fix it, so it has to be
 * visible and draggable while it is still wrong.
 */
export function rankStates(wf: WorkflowDoc): Map<string, number> {
  const out = new Map<string, number>()
  const adjacency = new Map<string, string[]>()
  for (const t of wf.transitions ?? []) {
    const list = adjacency.get(t.from) ?? []
    list.push(t.to)
    adjacency.set(t.from, list)
  }

  const start = wf.states.find((s) => s.id === wf.initial)?.id ?? wf.states[0]?.id
  if (start) {
    const queue: string[] = [start]
    out.set(start, 0)
    while (queue.length) {
      const node = queue.shift() as string
      const depth = out.get(node) ?? 0
      for (const next of adjacency.get(node) ?? []) {
        if (!out.has(next)) {
          out.set(next, depth + 1)
          queue.push(next)
        }
      }
    }
  }

  const maxRank = out.size ? Math.max(...out.values()) : 0
  for (const s of wf.states) {
    if (!out.has(s.id)) out.set(s.id, maxRank + 1)
  }
  return out
}

/**
 * Positions for every state in the document.
 *
 * `stored` positions win per node, so dragging one box does not forfeit the
 * automatic placement of the others — and a state added after someone laid the
 * graph out lands somewhere sensible instead of at the origin under another node.
 */
export function autoLayout(wf: WorkflowDoc, stored?: Layout | null): Layout {
  const ranks = rankStates(wf)
  const columns = new Map<number, string[]>()
  for (const s of wf.states) {
    const r = ranks.get(s.id) ?? 0
    const col = columns.get(r) ?? []
    col.push(s.id)
    columns.set(r, col)
  }

  const byId = new Map(wf.states.map((s) => [s.id, s]))
  const out: Layout = {}
  for (const [rank, ids] of [...columns.entries()].sort((a, b) => a[0] - b[0])) {
    const ordered = [...ids].sort((a, b) => {
      const ca = CATEGORY_ORDER[byId.get(a)?.category ?? ''] ?? 9
      const cb = CATEGORY_ORDER[byId.get(b)?.category ?? ''] ?? 9
      return ca - cb || a.localeCompare(b)
    })
    ordered.forEach((id, row) => {
      out[id] = stored?.[id] ?? {
        x: PAD + rank * (NODE_W + COL_GAP),
        y: PAD + row * (NODE_H + ROW_GAP),
      }
    })
  }
  return out
}

/** The canvas size needed to hold a layout, so the SVG can scroll rather than clip. */
export function layoutExtent(layout: Layout): { width: number; height: number } {
  const xs = Object.values(layout).map((p) => p.x)
  const ys = Object.values(layout).map((p) => p.y)
  return {
    width: (xs.length ? Math.max(...xs) : 0) + NODE_W + PAD,
    height: (ys.length ? Math.max(...ys) : 0) + NODE_H + PAD,
  }
}

/**
 * Where an edge should meet two boxes: the facing sides, vertically centred.
 *
 * Anchoring on the box EDGE rather than its centre is what keeps the arrowhead
 * outside the node instead of buried under it, and picking the side by relative
 * position is what makes a backwards edge (review → implementing, the one every
 * real workflow has) leave from the left rather than crossing through its own
 * source box.
 */
export function edgeAnchors(
  from: { x: number; y: number },
  to: { x: number; y: number },
): { x1: number; y1: number; x2: number; y2: number } {
  const forward = to.x >= from.x
  return {
    x1: forward ? from.x + NODE_W : from.x,
    y1: from.y + NODE_H / 2,
    x2: forward ? to.x : to.x + NODE_W,
    y2: to.y + NODE_H / 2,
  }
}

/** True when two states sit close enough that a straight edge would be hidden. */
export function isSelfish(from: { x: number; y: number }, to: { x: number; y: number }): boolean {
  return Math.abs(from.x - to.x) < 1 && Math.abs(from.y - to.y) < 1
}
