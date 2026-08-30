// A mindmap as a shared document.
//
// The map used to be rows, read over REST and refetched after every write. It is
// now a Y.Doc — the same machinery `/documents` runs, generalised rather than
// duplicated — so two people and an agent grow one map at the same time and see
// each other do it. `spec/mindmap-crdt.md` is the contract; this module is the
// browser half of it.
//
// Two top-level maps, both keyed by id and neither an array: an array makes a
// removal fight an insert, while keyed maps merge cleanly.
//
//   nodes         : Y.Map<nodeId, Y.Map>
//   relationships : Y.Map<relId,  Y.Map>
//
// `title` and `notes` are Y.Text because two people really do type into the same
// node, and last-write-wins would silently discard one of them. Everything else
// is a scalar and merges last-write-wins, which is the right answer for a colour
// or a coordinate. `style` is deliberately NOT a blob: a blob merges as a whole,
// so two people changing different things about one node clobber each other.
//
// The half of this file worth reading twice is [`normaliseNodes`]. A parent
// pointer under concurrency admits states no validator could have produced — two
// people each make a legal move that together form a cycle — so the tree is
// repaired deterministically on read, by a rule fixed hard enough that every peer
// computes the same tree from the same state.
import { between, isValid } from './fracdex'
import type { Point } from './mindmap-layout'

// ---- the shape ------------------------------------------------------------

export const NODE_KINDS = ['thought', 'question', 'decision', 'screen', 'component'] as const
export type NodeKind = (typeof NODE_KINDS)[number]

export const ATTACHMENT_KINDS = ['pdf', 'code', 'table', 'diagram', 'audio', 'link'] as const
export type AttachmentKind = (typeof ATTACHMENT_KINDS)[number]

export type NodeOrigin = 'human' | 'agent'

/** Caps. The API holds these too; the editor holds them because the server no
 *  longer sees an individual keystroke — a socket write arrives already merged. */
export const MAX_TITLE = 280
export const MAX_NOTES = 8000
export const MAX_NODES = 500
export const MAX_RELATIONSHIPS = 1000
export const MAX_ATTACHMENTS = 20
export const MAX_ICONS = 8
export const MAX_ATTACHMENT_NAME = 200
export const MAX_ATTACHMENT_GIST = 500
export const MAX_ATTACHMENT_REF = 2000
/** The label on the edge to a node's parent. Mirrors `MAX_EDGE_LABEL` in Rust. */
export const MAX_EDGE_LABEL = 80
/** The label on a relationship. Mirrors `MAX_REL_LABEL` in Rust. */
export const MAX_REL_LABEL = 80
/**
 * How deep a branch may go. Mirrors `MAX_DEPTH` in Rust.
 *
 * The canvas refuses a drop that would exceed it, so a drag cannot build a map
 * the API would then refuse to extend.
 */
export const MAX_DEPTH = 8

/** A pointer to something, never the thing. See the spec: bytes in a CRDT update
 *  log are bytes every peer replays on join. */
export interface Attachment {
  id: string
  kind: AttachmentKind
  name: string
  gist: string
  ref: string
}

export interface Promotion {
  kind: 'epic' | 'initiative'
  id: string
}

/** A node exactly as the document holds it, before the tree is normalised. */
export interface RawNode {
  id: string
  parent: string | null
  /** Fractional index. Siblings sort by it, ties broken by id. */
  order: string
  title: string
  notes: string
  /** Hand placement. Both coordinates or neither — half a coordinate places nothing. */
  at: Point | null
  /** Labels the edge to this node's PARENT, which is unambiguous: a node has one. */
  edge_label: string
  kind: NodeKind
  origin: NodeOrigin
  reviewed: boolean
  icons: string[]
  color: string
  shape: string
  attachments: Attachment[]
  promoted: Promotion | null
  created_by: string
  /** Epoch milliseconds, the representation the server stores. */
  created_at: number
  /** Epoch milliseconds, the representation the server stores. */
  updated_at: number
}

/** A node after normalisation: parent repaired, sibling rank assigned. */
export interface MapNode extends RawNode {
  /** Integer sibling rank derived from the sorted order — what the layout reads. */
  position: number
}

/** The scalar fields a details panel edits. Last-write-wins each, independently,
 *  which is exactly why they are separate keys and not one style blob. */
export interface NodeFields {
  edge_label: string
  kind: NodeKind
  color: string
  shape: string
  reviewed: boolean
}

/** An edge that is NOT part of the hierarchy. */
export interface Relationship {
  id: string
  from: string
  to: string
  label: string
}

// ---- normalisation --------------------------------------------------------

/**
 * Break every cycle, choosing the node that moves by the fixed rule.
 *
 * Where a cycle has several members the LOWEST id is the one re-parented to
 * root, so the outcome does not depend on which node the walk happened to start
 * from — two peers holding the same state must draw the same tree.
 *
 * Mutates `parents` in place.
 */
function breakCycles(parents: Map<string, string | null>): void {
  for (const start of [...parents.keys()].sort()) {
    const seen = new Set<string>()
    let cursor: string | null = start
    while (cursor !== null) {
      if (seen.has(cursor)) {
        // `cursor` is on the loop, so walking from it returns to it.
        const cycle: string[] = []
        let step: string = cursor
        do {
          cycle.push(step)
          step = parents.get(step) as string
        } while (step !== cursor)
        parents.set([...cycle].sort()[0] as string, null)
        break
      }
      seen.add(cursor)
      cursor = parents.get(cursor) ?? null
    }
  }
}

/** Sibling order: valid keys first and in key order, invalid keys last, id breaks ties. */
export function compareSiblings(a: RawNode, b: RawNode): number {
  const av = isValid(a.order)
  const bv = isValid(b.order)
  // An order key this module could not have produced sorts last rather than
  // anywhere: a peer is not a trusted writer, and a malformed key must not make
  // sibling order depend on who read it.
  if (av !== bv) return av ? -1 : 1
  if (av && a.order !== b.order) return a.order < b.order ? -1 : 1
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
}

/**
 * The tree every peer agrees on, from whatever the document happens to hold.
 *
 * Three repairs, all deterministic and all pure:
 *
 *   1. a node whose ancestry loops is re-parented to root, lowest id first;
 *   2. a node whose parent no longer exists is re-parented to root, never
 *      dropped — losing a subtree because somebody deleted its parent
 *      concurrently is worse than showing it;
 *   3. siblings are ordered by fractional index with the id tiebreak, and given
 *      the integer rank the layout consumes.
 *
 * Returned depth-first, parents before children.
 */
export function normaliseNodes(raw: readonly RawNode[]): MapNode[] {
  const known = new Set(raw.map((n) => n.id))
  const parents = new Map<string, string | null>()
  for (const n of raw) {
    parents.set(n.id, n.parent !== null && known.has(n.parent) ? n.parent : null)
  }
  breakCycles(parents)

  const kids = new Map<string | null, RawNode[]>()
  for (const n of raw) {
    const parent = parents.get(n.id) ?? null
    const list = kids.get(parent)
    if (list) list.push(n)
    else kids.set(parent, [n])
  }
  for (const list of kids.values()) list.sort(compareSiblings)

  const out: MapNode[] = []
  const walk = (parent: string | null): void => {
    const list = kids.get(parent) ?? []
    list.forEach((n, index) => {
      out.push({ ...n, parent, position: index })
      walk(n.id)
    })
  }
  walk(null)
  return out
}

/** A relationship whose end no longer resolves is dropped: a dangling edge is not
 *  a node and there is nothing to keep. */
export function normaliseRelationships(
  raw: readonly Relationship[],
  nodes: readonly { id: string }[],
): Relationship[] {
  const known = new Set(nodes.map((n) => n.id))
  return raw
    .filter((r) => known.has(r.from) && known.has(r.to))
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
}

/** Every id beneath `id`, not including it. */
/**
 * How many descendants each node has, in one pass.
 *
 * The obvious version — ask `descendantsOf` once per node — rebuilds the whole
 * parent→children index every time, so a 500-node map costs 500 rebuilds of a
 * 500-entry map. That runs on every render, and a render happens on every
 * character a collaborator types.
 */
export function descendantCounts(nodes: readonly MapNode[]): Map<string, number> {
  const kids = new Map<string, string[]>()
  for (const n of nodes) {
    if (n.parent === null) continue
    const list = kids.get(n.parent)
    if (list) list.push(n.id)
    else kids.set(n.parent, [n.id])
  }
  const counts = new Map<string, number>()
  // Post-order: a node's total is its children plus each child's own total, and
  // every child is finished before its parent is asked.
  const walk = (id: string): number => {
    let total = 0
    for (const child of kids.get(id) ?? []) total += 1 + walk(child)
    counts.set(id, total)
    return total
  }
  for (const n of nodes) {
    if (n.parent === null) walk(n.id)
  }
  return counts
}

/**
 * Every node on the path from the root down to `id`, root first, excluding it.
 *
 * "Go to node…" needs this: a match can be inside a branch this viewer folded,
 * and jumping to a node that is not drawn is worse than not jumping. Unfolding
 * the ancestors is what makes the destination visible — and it changes only this
 * viewer's fold, never the document.
 */
export function ancestorsOf(nodes: readonly MapNode[], id: string): string[] {
  const parents = new Map(nodes.map((n) => [n.id, n.parent]))
  const out: string[] = []
  const seen = new Set<string>([id])
  let cursor = parents.get(id) ?? null
  // `normaliseNodes` has already broken every cycle, but this walks whatever it
  // is handed: a guard costs one Set and cannot hang the render.
  while (cursor !== null && !seen.has(cursor)) {
    seen.add(cursor)
    out.push(cursor)
    cursor = parents.get(cursor) ?? null
  }
  return out.reverse()
}

export function descendantsOf(nodes: readonly MapNode[], id: string): string[] {
  const kids = new Map<string, string[]>()
  for (const n of nodes) {
    if (n.parent === null) continue
    const list = kids.get(n.parent)
    if (list) list.push(n.id)
    else kids.set(n.parent, [n.id])
  }
  const out: string[] = []
  const walk = (from: string): void => {
    for (const child of kids.get(from) ?? []) {
      out.push(child)
      walk(child)
    }
  }
  walk(id)
  return out
}

/**
 * The nodes a viewer can see, given the branches THEY have collapsed.
 *
 * Fold is per-viewer and lives in browser storage beside pan and zoom — never in
 * the document. Collapsing a branch must not collapse it under somebody else
 * mid-conversation.
 */
export function visibleNodes(
  nodes: readonly MapNode[],
  collapsed: ReadonlySet<string>,
): MapNode[] {
  if (collapsed.size === 0) return [...nodes]
  const hidden = new Set<string>()
  for (const id of collapsed) {
    if (nodes.some((n) => n.id === id)) for (const d of descendantsOf(nodes, id)) hidden.add(d)
  }
  return nodes.filter((n) => !hidden.has(n.id))
}

/** How many thoughts a collapsed branch is holding back. */
export function hiddenCount(nodes: readonly MapNode[], id: string): number {
  return descendantsOf(nodes, id).length
}

/**
 * A fractional index that lands after `afterId` and before whatever follows it.
 *
 * The one piece of write-side arithmetic worth testing on its own: gapped
 * integers cannot survive concurrency — two peers inserting between the same pair
 * both pick 1500 — and this is what replaces them. An unusable neighbouring key
 * is ignored rather than built on.
 */
export function orderBetween(
  siblings: readonly { id: string; order: string }[],
  afterId: string | null,
): string {
  const valid = siblings.map((s) => s.order).filter(isValid)
  const last = valid.length > 0 ? (valid[valid.length - 1] as string) : null
  if (afterId === null) return between(last, null)
  const index = siblings.findIndex((s) => s.id === afterId)
  if (index === -1) return between(last, null)
  const before = siblings[index] as { order: string }
  const after = siblings[index + 1]
  return between(
    isValid(before.order) ? before.order : null,
    after && isValid(after.order) ? after.order : null,
  )
}
