// The writes that are not one `set` — asking something and answering it, and
// cutting a node loose from its parent.
//
// Driven against a real `Y.Doc`, because the interesting part is what the
// document holds afterwards: the answer landed on the OTHER node, the question is
// gone, and the relation went with it. Every other write in `mindmap-crdt.ts` is
// a field assignment whose behaviour is `mindmap-doc.ts`'s to describe.
import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'

import {
  answerQuestion,
  fragmentText,
  proseOf,
  proseTextOf,
  createNode,
  createQuestion,
  detach,
  readNodes,
  readRelationships,
  setNotes,
} from './mindmap-crdt'

const fresh = () => new Y.Doc()

const nodeById = (doc: Y.Doc, id: string) => readNodes(doc).find((n) => n.id === id) ?? null

describe('createQuestion', () => {
  it('makes a first-ring question node related to what it doubts', () => {
    const doc = fresh()
    const about = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    const q = createQuestion(doc, about, 'Per seat or per project?', 'me')!

    const question = nodeById(doc, q)!
    expect(question.kind).toBe('question')
    expect(question.parent).toBeNull()
    expect(question.title).toBe('Per seat or per project?')

    const rels = readRelationships(doc, readNodes(doc))
    expect(rels).toHaveLength(1)
    expect(rels[0]).toMatchObject({ from: q, to: about })
  })

  it('will pose a question about nothing in particular', () => {
    const doc = fresh()
    const q = createQuestion(doc, null, 'Who owns this?', 'me')!
    expect(nodeById(doc, q)?.kind).toBe('question')
    expect(readRelationships(doc, readNodes(doc))).toEqual([])
  })
})

describe('answerQuestion', () => {
  it('appends the answer to the related node, marks it looked at, and removes the question', () => {
    const doc = fresh()
    const about = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    setNotes(doc, about, 'Two models on the table.')
    const q = createQuestion(doc, about, 'Per seat or per project?', 'me')!

    expect(answerQuestion(doc, q, '  Per project, for now.  ')).toBe(about)

    const target = nodeById(doc, about)!
    // One line per BLOCK, not per keystroke of whitespace: a section's prose is
    // an XmlFragment of paragraphs now, and a blank line is not a paragraph.
    // `appendAnswer` still separates the answer with one, and the round trip
    // through the fragment reads it back as two blocks — which is what the API
    // returns as `notes` and what a card shows.
    expect(target.notes).toBe('Two models on the table.\nPer project, for now.')
    expect(target.reviewed).toBe(true)
    expect(nodeById(doc, q)).toBeNull()
    // The relation went with it: an edge to a node that is gone is not an edge.
    expect(readRelationships(doc, readNodes(doc))).toEqual([])
  })

  it('keeps the answer on the question itself when it questions nothing', () => {
    const doc = fresh()
    const q = createQuestion(doc, null, 'Who owns this?', 'me')!

    expect(answerQuestion(doc, q, 'Design does.')).toBe(q)

    const answered = nodeById(doc, q)!
    expect(answered.notes).toBe('Design does.')
    expect(answered.reviewed).toBe(true)
    // It stops being a question, because it is not one any more.
    expect(answered.kind).toBe('thought')
  })

  it('does nothing at all for a blank answer or a question that is gone', () => {
    const doc = fresh()
    const about = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    const q = createQuestion(doc, about, 'Which?', 'me')!

    expect(answerQuestion(doc, q, '   ')).toBeNull()
    expect(nodeById(doc, q)).not.toBeNull()
    expect(nodeById(doc, about)?.reviewed).toBe(false)

    expect(answerQuestion(doc, 'mn-nope', 'anything')).toBeNull()
  })
})

describe('detach', () => {
  it('cuts the child loose without moving or removing anything', () => {
    const doc = fresh()
    const parent = createNode(doc, { parent: null, title: 'Root', by: 'me' })!
    const child = createNode(doc, { parent, title: 'Kid', by: 'me' })!
    const grandchild = createNode(doc, { parent: child, title: 'Grandkid', by: 'me' })!

    detach(doc, child, { x: 12, y: 34 })

    expect(nodeById(doc, child)?.parent).toBeNull()
    // Pinned where it was drawn, so the map does not jump under the reader.
    expect(nodeById(doc, child)?.at).toEqual({ x: 12, y: 34 })
    // Everything under it came along.
    expect(nodeById(doc, grandchild)?.parent).toBe(child)
    expect(readNodes(doc)).toHaveLength(3)
  })
})


describe('prose', () => {
  it('gives a new node a fragment an editor can bind to', () => {
    // The whole of phase 3 rests on this: a section's prose is a nested
    // XmlFragment in the node's own map, so `/documents` can point an editor at
    // it without a second document existing anywhere.
    const doc = fresh()
    const id = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    const frag = doc.getMap<Y.Map<unknown>>('nodes').get(id)?.get('prose')
    expect(frag).toBeInstanceOf(Y.XmlFragment)
  })

  it('reads a section back one line per block, the way the API does', () => {
    const doc = fresh()
    const id = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    setNotes(doc, id, 'Two models.\n\nNeither is settled.')
    expect(proseTextOf(doc, id)).toBe('Two models.\nNeither is settled.')
    expect(nodeById(doc, id)?.notes).toBe('Two models.\nNeither is settled.')
  })

  it('gives every block an id, so an agent can address one', () => {
    const doc = fresh()
    const id = createNode(doc, { parent: null, title: 'Pricing', by: 'me' })!
    setNotes(doc, id, 'one\ntwo')
    const frag = proseOf(doc, id)!
    const ids = frag.toArray().map((el) => (el as Y.XmlElement).getAttribute('id'))
    expect(ids.every((x) => typeof x === 'string' && x.startsWith('blk_'))).toBe(true)
    expect(new Set(ids).size).toBe(2)
  })

  it('migrates a node written before prose existed, without losing its notes', () => {
    // A map made by an older build carries `notes` as a Y.Text. It stays legible
    // until something touches it, and touching it moves it into the fragment
    // rather than leaving one paragraph in two places.
    const doc = fresh()
    const nodes = doc.getMap<Y.Map<unknown>>('nodes')
    nodes.set('mn-old', new Y.Map())
    const entry = nodes.get('mn-old') as Y.Map<unknown>
    entry.set('parent', null)
    entry.set('order', 'a0')
    entry.set('title', new Y.Text('Legacy'))
    entry.set('notes', new Y.Text('Written before prose.'))

    expect(nodeById(doc, 'mn-old')?.notes).toBe('Written before prose.')

    const frag = proseOf(doc, 'mn-old')!
    expect(fragmentText(frag)).toBe('Written before prose.')
  })

  it('does not make a fragment just because somebody read the node', () => {
    const doc = fresh()
    const nodes = doc.getMap<Y.Map<unknown>>('nodes')
    nodes.set('mn-old', new Y.Map())
    const entry = nodes.get('mn-old') as Y.Map<unknown>
    entry.set('parent', null)
    entry.set('order', 'a0')
    entry.set('title', new Y.Text('Legacy'))

    expect(proseTextOf(doc, 'mn-old')).toBe('')
    expect(entry.get('prose')).toBeUndefined()
  })
})
