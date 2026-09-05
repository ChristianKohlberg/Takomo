// The half of a collaborative mindmap that can be proven.
//
// jsdom has no layout engine and no socket, so nothing here is about how the map
// looks or whether it syncs. What these cover is the part concurrency makes
// genuinely hard: a shared document can hold a tree no synchronous validator
// could ever have accepted, and every peer has to derive the SAME picture from
// it. If normalisation is not deterministic, two people looking at one map see
// two different maps and neither is wrong.
import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'

import {
  MAX_NOTES,
  MAX_TITLE,
  ancestorsOf,
  compareSiblings,
  descendantsOf,
  descendantCounts,
  hiddenCount,
  normaliseNodes,
  normaliseRelationships,
  orderBetween,
  visibleNodes,
  type RawNode,
} from './mindmap-doc'
import {
  applyText,
  createNode,
  createRelationship,
  addAttachment,
  deleteRelationship,
  deleteSubtree,
  place,
  readNodes,
  readRelationships,
  reparent,
  setNotes,
  setTitle,
  tidyAll,
} from './mindmap-crdt'
import { first, between } from './fracdex'

function raw(over: Partial<RawNode> & { id: string }): RawNode {
  return {
    parent: null,
    order: 'V',
    title: 'a thought',
    notes: '',
    at: null,
    edge_label: '',
    kind: 'thought',
    origin: 'human',
    reviewed: false,
    icons: [],
    color: '',
    shape: '',
    attachments: [],
    promoted: null,
    created_by: 'tok-1',
    created_at: 1_767_225_600_000,
    updated_at: 1_767_225_600_000,
    ...over,
  }
}

/** Write a node straight into the document, bypassing every writer. This is how
 *  a state that could not be produced legally gets constructed for a test. */
function put(doc: Y.Doc, r: RawNode): void {
  const nodes = doc.getMap<Y.Map<unknown>>('nodes')
  nodes.set(r.id, new Y.Map())
  const m = nodes.get(r.id) as Y.Map<unknown>
  m.set('parent', r.parent)
  m.set('order', r.order)
  m.set('title', new Y.Text(r.title))
  m.set('x', r.at ? r.at.x : null)
  m.set('y', r.at ? r.at.y : null)
}

describe('normaliseNodes — cycles', () => {
  it('re-parents the lowest id in a cycle, so both peers draw the same tree', () => {
    // A under B while B goes under A: two legal moves that are illegal together,
    // and nothing server-side can refuse them any more.
    const tree = normaliseNodes([raw({ id: 'b', parent: 'a' }), raw({ id: 'a', parent: 'b' })])
    expect(tree.find((n) => n.id === 'a')!.parent).toBeNull()
    expect(tree.find((n) => n.id === 'b')!.parent).toBe('a')
  })

  it('does not depend on the order the nodes arrive in', () => {
    const forwards = normaliseNodes([
      raw({ id: 'c', parent: 'a' }),
      raw({ id: 'a', parent: 'b' }),
      raw({ id: 'b', parent: 'c' }),
    ])
    const backwards = normaliseNodes([
      raw({ id: 'b', parent: 'c' }),
      raw({ id: 'a', parent: 'b' }),
      raw({ id: 'c', parent: 'a' }),
    ])
    const shape = (t: ReturnType<typeof normaliseNodes>) =>
      [...t].sort((x, y) => x.id.localeCompare(y.id)).map((n) => `${n.id}<-${n.parent}`)
    expect(shape(forwards)).toEqual(shape(backwards))
    // …and the lowest id is the one that moved.
    expect(forwards.find((n) => n.id === 'a')!.parent).toBeNull()
  })

  it('breaks two separate cycles independently', () => {
    const tree = normaliseNodes([
      raw({ id: 'a', parent: 'b' }),
      raw({ id: 'b', parent: 'a' }),
      raw({ id: 'x', parent: 'y' }),
      raw({ id: 'y', parent: 'x' }),
    ])
    expect(tree.find((n) => n.id === 'a')!.parent).toBeNull()
    expect(tree.find((n) => n.id === 'x')!.parent).toBeNull()
    expect(tree).toHaveLength(4)
  })

  it('keeps a node that merely points INTO a cycle where it is', () => {
    const tree = normaliseNodes([
      raw({ id: 'a', parent: 'b' }),
      raw({ id: 'b', parent: 'a' }),
      raw({ id: 'z', parent: 'b' }),
    ])
    expect(tree.find((n) => n.id === 'z')!.parent).toBe('b')
  })
})

describe('normaliseNodes — orphans', () => {
  it('lifts a node whose parent is gone to the root rather than dropping it', () => {
    // Losing a subtree because somebody deleted its parent concurrently is worse
    // than showing it at the top level.
    const tree = normaliseNodes([raw({ id: 'kid', parent: 'ghost' })])
    expect(tree).toHaveLength(1)
    expect(tree[0]!.parent).toBeNull()
  })

  it('keeps the orphan’s own children under it', () => {
    const tree = normaliseNodes([
      raw({ id: 'kid', parent: 'ghost' }),
      raw({ id: 'grandkid', parent: 'kid' }),
    ])
    expect(tree.find((n) => n.id === 'grandkid')!.parent).toBe('kid')
  })
})

describe('normaliseNodes — sibling order', () => {
  it('orders by fractional index and breaks ties by id', () => {
    const tree = normaliseNodes([
      raw({ id: 'c', order: 'V' }),
      raw({ id: 'a', order: 'V' }),
      raw({ id: 'b', order: 'H' }),
    ])
    expect(tree.map((n) => n.id)).toEqual(['b', 'a', 'c'])
    expect(tree.map((n) => n.position)).toEqual([0, 1, 2])
  })

  it('sorts an unusable order key last, by id, instead of anywhere', () => {
    // A peer is not a trusted writer. A key this module could not have produced
    // must not make sibling order depend on who read the document.
    const tree = normaliseNodes([
      raw({ id: 'junk-b', order: '!!' }),
      raw({ id: 'junk-a', order: '' }),
      raw({ id: 'good', order: 'V' }),
    ])
    expect(tree.map((n) => n.id)).toEqual(['good', 'junk-a', 'junk-b'])
  })

  it('ranks each ring from zero', () => {
    const tree = normaliseNodes([
      raw({ id: 'p', order: 'V' }),
      raw({ id: 'k2', parent: 'p', order: 'k' }),
      raw({ id: 'k1', parent: 'p', order: 'V' }),
    ])
    expect(tree.filter((n) => n.parent === 'p').map((n) => [n.id, n.position])).toEqual([
      ['k1', 0],
      ['k2', 1],
    ])
  })

  it('returns parents before their children', () => {
    const tree = normaliseNodes([
      raw({ id: 'kid', parent: 'top' }),
      raw({ id: 'top' }),
    ])
    expect(tree.map((n) => n.id)).toEqual(['top', 'kid'])
  })
})

describe('normaliseRelationships', () => {
  const nodes = normaliseNodes([raw({ id: 'a' }), raw({ id: 'b' })])

  it('keeps an edge whose ends both resolve', () => {
    expect(
      normaliseRelationships([{ id: 'r1', from: 'a', to: 'b', label: 'see also' }], nodes),
    ).toHaveLength(1)
  })

  it('drops a dangling edge rather than repairing it', () => {
    // A dangling edge is not a node; there is nothing to keep.
    expect(
      normaliseRelationships(
        [
          { id: 'r1', from: 'a', to: 'gone', label: '' },
          { id: 'r2', from: 'gone', to: 'b', label: '' },
        ],
        nodes,
      ),
    ).toEqual([])
  })
})

describe('compareSiblings', () => {
  it('is a total order', () => {
    const list = [raw({ id: 'b', order: 'V' }), raw({ id: 'a', order: 'V' })]
    expect(compareSiblings(list[0]!, list[1]!)).toBe(1)
    expect(compareSiblings(list[1]!, list[0]!)).toBe(-1)
    expect(compareSiblings(list[0]!, list[0]!)).toBe(0)
  })
})

describe('orderBetween', () => {
  it('appends past the end of the ring', () => {
    const a = first()
    const key = orderBetween([{ id: 'x', order: a }], null)
    expect(key > a).toBe(true)
  })

  it('lands strictly between two siblings', () => {
    const a = first()
    const b = between(a, null)
    const key = orderBetween(
      [
        { id: 'x', order: a },
        { id: 'y', order: b },
      ],
      'x',
    )
    expect(a < key && key < b).toBe(true)
  })

  it('appends when the anchor is not in the ring', () => {
    const a = first()
    expect(orderBetween([{ id: 'x', order: a }], 'ghost') > a).toBe(true)
  })

  it('ignores an unusable neighbouring key rather than building on it', () => {
    const key = orderBetween([{ id: 'x', order: '!!' }], 'x')
    expect(key.length).toBeGreaterThan(0)
  })
})

describe('descendantsOf / visibleNodes / hiddenCount', () => {
  it('counts every branch in one pass, agreeing with the simple version', () => {
    // `descendantCounts` exists because asking `hiddenCount` per node rebuilds
    // the child index each time — 500 rebuilds of a 500-entry map, on every
    // remote keystroke. It must give exactly the same answers.
    const tree = normaliseNodes([
      raw({ id: 'top' }),
      raw({ id: 'a', parent: 'top' }),
      raw({ id: 'b', parent: 'top' }),
      raw({ id: 'a1', parent: 'a' }),
      raw({ id: 'a1x', parent: 'a1' }),
      raw({ id: 'lonely' }),
    ])
    const fast = descendantCounts(tree)
    for (const node of tree) {
      expect(fast.get(node.id) ?? 0, node.id).toBe(hiddenCount(tree, node.id))
    }
    expect(fast.get('top')).toBe(4)
    expect(fast.get('a')).toBe(2)
    expect(fast.get('lonely')).toBe(0)
  })

  const tree = normaliseNodes([
    raw({ id: 'top' }),
    raw({ id: 'mid', parent: 'top' }),
    raw({ id: 'leaf', parent: 'mid' }),
    raw({ id: 'other' }),
  ])

  it('walks the whole subtree', () => {
    expect(descendantsOf(tree, 'top').sort()).toEqual(['leaf', 'mid'])
    expect(descendantsOf(tree, 'leaf')).toEqual([])
  })

  it('hides a collapsed branch from the viewer who collapsed it, and nobody else', () => {
    // Fold is per-viewer state — it is a filter over the shared tree, not a fact
    // in it, so collapsing here cannot collapse anything under a collaborator.
    const shown = visibleNodes(tree, new Set(['top']))
    expect(shown.map((n) => n.id)).toEqual(['other', 'top'])
    expect(visibleNodes(tree, new Set()).map((n) => n.id)).toEqual(tree.map((n) => n.id))
  })

  it('counts what a fold is holding back', () => {
    expect(hiddenCount(tree, 'top')).toBe(2)
  })
})

describe('applyText', () => {
  it('touches only what changed, so a collaborator’s caret survives', () => {
    const doc = new Y.Doc()
    const text = doc.getText('t')
    text.insert(0, 'the API versioning question')
    applyText(text, 'the API versioning answer')
    expect(text.toString()).toBe('the API versioning answer')
  })

  it('is a no-op when nothing changed', () => {
    const doc = new Y.Doc()
    const text = doc.getText('t')
    text.insert(0, 'same')
    let updates = 0
    doc.on('update', () => (updates += 1))
    applyText(text, 'same')
    expect(updates).toBe(0)
  })

  it('handles insertion in the middle', () => {
    const doc = new Y.Doc()
    const text = doc.getText('t')
    text.insert(0, 'ab')
    applyText(text, 'aXb')
    expect(text.toString()).toBe('aXb')
  })
})

describe('reading a real document', () => {
  it('round-trips a node written by the writers', () => {
    const doc = new Y.Doc()
    const id = createNode(doc, { parent: null, title: 'versioning?', by: 'tok-1' })!
    const nodes = readNodes(doc)
    expect(nodes).toHaveLength(1)
    expect(nodes[0]!.title).toBe('versioning?')
    expect(nodes[0]!.kind).toBe('thought')
    expect(nodes[0]!.origin).toBe('human')
    expect(nodes[0]!.at).toBeNull()
    expect(nodes[0]!.position).toBe(0)
    expect(id.startsWith('mn-')).toBe(true)
  })

  it('keeps siblings in the order they were made', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    const b = createNode(doc, { parent: null, title: 'b', by: 'x' })!
    const c = createNode(doc, { parent: null, title: 'c', after: a, by: 'x' })!
    expect(readNodes(doc).map((n) => n.id)).toEqual([a, c, b])
  })

  it('normalises what it reads, not just what it was given', () => {
    const doc = new Y.Doc()
    put(doc, raw({ id: 'a', parent: 'b' }))
    put(doc, raw({ id: 'b', parent: 'a' }))
    expect(readNodes(doc).find((n) => n.id === 'a')!.parent).toBeNull()
  })

  it('reads a half-written record without blanking the map', () => {
    // A peer may have written a partial node; a missing key must default, not
    // throw.
    const doc = new Y.Doc()
    doc.getMap<Y.Map<unknown>>('nodes').set('bare', new Y.Map())
    const nodes = readNodes(doc)
    expect(nodes).toHaveLength(1)
    expect(nodes[0]!.title).toBe('')
    expect(nodes[0]!.parent).toBeNull()
  })
})

describe('writing', () => {
  it('caps a title at 280 characters', () => {
    const doc = new Y.Doc()
    const id = createNode(doc, { parent: null, title: 'x'.repeat(400), by: 'x' })!
    expect(readNodes(doc)[0]!.title).toHaveLength(MAX_TITLE)
    setTitle(doc, id, 'y'.repeat(400))
    expect(readNodes(doc)[0]!.title).toHaveLength(MAX_TITLE)
  })

  it('caps notes at 8000 and keeps them off the title', () => {
    const doc = new Y.Doc()
    const id = createNode(doc, { parent: null, title: 'short', by: 'x' })!
    setNotes(doc, id, 'n'.repeat(MAX_NOTES + 50))
    const node = readNodes(doc)[0]!
    expect(node.notes).toHaveLength(MAX_NOTES)
    expect(node.title).toBe('short')
  })

  it('takes the whole subtree when a node is deleted', () => {
    const doc = new Y.Doc()
    const top = createNode(doc, { parent: null, title: 'top', by: 'x' })!
    const mid = createNode(doc, { parent: top, title: 'mid', by: 'x' })!
    createNode(doc, { parent: mid, title: 'leaf', by: 'x' })
    const other = createNode(doc, { parent: null, title: 'other', by: 'x' })!
    deleteSubtree(doc, top)
    expect(readNodes(doc).map((n) => n.id)).toEqual([other])
  })

  it('takes the relationships that touched it too', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    const b = createNode(doc, { parent: null, title: 'b', by: 'x' })!
    createRelationship(doc, a, b, 'contradicts')
    expect(readRelationships(doc, readNodes(doc))).toHaveLength(1)
    deleteSubtree(doc, b)
    expect(readRelationships(doc, readNodes(doc))).toEqual([])
  })

  it('refuses a relationship from a node to itself', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    expect(createRelationship(doc, a, a, 'loop')).toBeNull()
  })

  it('deletes one relationship without touching the others', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    const b = createNode(doc, { parent: null, title: 'b', by: 'x' })!
    const one = createRelationship(doc, a, b, 'one')!
    createRelationship(doc, b, a, 'two')
    deleteRelationship(doc, one)
    expect(readRelationships(doc, readNodes(doc)).map((r) => r.label)).toEqual(['two'])
  })

  it('un-pins a node when it is dropped onto a new parent', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    const b = createNode(doc, { parent: null, title: 'b', by: 'x' })!
    place(doc, b, { x: 400, y: -120 })
    expect(readNodes(doc).find((n) => n.id === b)!.at).toEqual({ x: 400, y: -120 })
    reparent(doc, b, a)
    const moved = readNodes(doc).find((n) => n.id === b)!
    expect(moved.parent).toBe(a)
    expect(moved.at).toBeNull()
  })

  it('hands every node back to the layout on tidy', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    place(doc, a, { x: 10, y: 10 })
    tidyAll(doc)
    expect(readNodes(doc)[0]!.at).toBeNull()
  })

  it('stores an attachment as a pointer, never as bytes', () => {
    const doc = new Y.Doc()
    const a = createNode(doc, { parent: null, title: 'a', by: 'x' })!
    addAttachment(doc, a, {
      kind: 'pdf',
      name: 'pricing.pdf',
      gist: 'the 2026 rate card',
      ref: 'https://example.invalid/pricing.pdf',
    })
    const [att] = readNodes(doc)[0]!.attachments
    expect(att!.kind).toBe('pdf')
    expect(att!.ref).toBe('https://example.invalid/pricing.pdf')
  })
})

describe('two peers', () => {
  it('converge on one tree after exchanging updates', () => {
    const alice = new Y.Doc()
    const bob = new Y.Doc()
    const sync = () => {
      Y.applyUpdate(bob, Y.encodeStateAsUpdate(alice, Y.encodeStateVector(bob)))
      Y.applyUpdate(alice, Y.encodeStateAsUpdate(bob, Y.encodeStateVector(alice)))
    }
    const root = createNode(alice, { parent: null, title: 'API', by: 'a' })!
    sync()
    // Each adds a child without seeing the other's, then they meet.
    createNode(alice, { parent: root, title: 'versioning', by: 'a' })
    createNode(bob, { parent: root, title: 'retries', by: 'b' })
    sync()
    const left = readNodes(alice).map((n) => `${n.id}:${n.title}`)
    const right = readNodes(bob).map((n) => `${n.id}:${n.title}`)
    expect(left).toEqual(right)
    expect(left).toHaveLength(3)
  })

  it('resolves the cycle two concurrent moves create, identically on both sides', () => {
    const alice = new Y.Doc()
    const bob = new Y.Doc()
    const a = createNode(alice, { parent: null, title: 'a', by: 'a' })!
    const b = createNode(alice, { parent: null, title: 'b', by: 'a' })!
    Y.applyUpdate(bob, Y.encodeStateAsUpdate(alice))
    // A under B here, B under A there.
    reparent(alice, a, b)
    reparent(bob, b, a)
    Y.applyUpdate(bob, Y.encodeStateAsUpdate(alice))
    Y.applyUpdate(alice, Y.encodeStateAsUpdate(bob))
    const shape = (doc: Y.Doc) =>
      readNodes(doc)
        .map((n) => `${n.id}<-${n.parent}`)
        .sort()
    expect(shape(alice)).toEqual(shape(bob))
    // Exactly one of them ends up at the root, and both peers agree which.
    expect(readNodes(alice).filter((n) => n.parent === null)).toHaveLength(1)
  })
})

describe('ancestorsOf', () => {
  const tree = normaliseNodes([
    raw({ id: 'a' }),
    raw({ id: 'b', parent: 'a' }),
    raw({ id: 'c', parent: 'b' }),
    raw({ id: 'd' }),
  ])

  it('walks root-first to the node, excluding it', () => {
    expect(ancestorsOf(tree, 'c')).toEqual(['a', 'b'])
  })

  it('returns nothing for a root', () => {
    expect(ancestorsOf(tree, 'd')).toEqual([])
  })

  it('is safe on a node that is not there', () => {
    expect(ancestorsOf(tree, 'nope')).toEqual([])
  })
})
