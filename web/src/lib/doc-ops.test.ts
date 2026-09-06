import { describe, expect, it } from 'vitest'
import { Schema, type Node as PMNode } from '@tiptap/pm/model'
import { EditorState } from '@tiptap/pm/state'

import { applyOps, markdownToNodes, parseProposal, touchedBlocks, type Proposal } from './doc-ops'

// Close enough to StarterKit for these to mean something: the node names and the
// `id` attribute are what the ops address.
const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    text: { group: 'inline' },
    paragraph: { group: 'block', content: 'inline*', attrs: { id: { default: null } } },
    heading: {
      group: 'block',
      content: 'inline*',
      attrs: { id: { default: null }, level: { default: 1 } },
    },
    codeBlock: {
      group: 'block',
      content: 'text*',
      attrs: { id: { default: null }, language: { default: null } },
    },
    blockquote: { group: 'block', content: 'block+', attrs: { id: { default: null } } },
    horizontalRule: { group: 'block', attrs: { id: { default: null } } },
    bulletList: { group: 'block', content: 'listItem+', attrs: { id: { default: null } } },
    orderedList: { group: 'block', content: 'listItem+', attrs: { id: { default: null } } },
    listItem: { content: 'paragraph+' },
  },
})

const para = (id: string, text: string) =>
  schema.nodes.paragraph!.create({ id }, schema.text(text))

/** The text of each top-level block, in order. */
function texts(doc: PMNode): string[] {
  const out: string[] = []
  doc.forEach((n) => out.push(n.textContent))
  return out
}

function stateWith(...nodes: ReturnType<typeof para>[]) {
  return EditorState.create({ schema, doc: schema.nodes.doc!.create(null, nodes) })
}

describe('markdownToNodes', () => {
  it('preserves Mermaid fences with blank lines between adjacent prose blocks', () => {
    const nodes = markdownToNodes(schema, 'Before.\n```mermaid\nflowchart TD\n\n  A --> B\n```\nAfter.')
    expect(nodes.map((node) => node.type.name)).toEqual(['paragraph', 'codeBlock', 'paragraph'])
    expect(nodes[1]?.attrs.language).toBe('mermaid')
    expect(nodes[1]?.textContent).toBe('flowchart TD\n\n  A --> B')
  })

  it('reads a heading at the level its hashes say', () => {
    const [n] = markdownToNodes(schema, '### Pricing')
    expect(n!.type.name).toBe('heading')
    expect(n!.attrs.level).toBe(3)
    expect(n!.textContent).toBe('Pricing')
  })

  it('splits blocks on blank lines, so one op can carry several', () => {
    const nodes = markdownToNodes(schema, 'First.\n\nSecond.')
    expect(nodes).toHaveLength(2)
    expect(nodes.map((n) => n.textContent)).toEqual(['First.', 'Second.'])
  })

  it('does not read the inside of a fenced code block as markdown', () => {
    const [n] = markdownToNodes(schema, '```sh\n# not a heading\n```')
    expect(n!.type.name).toBe('codeBlock')
    expect(n!.textContent).toBe('# not a heading')
  })

  it('builds a list with its items intact', () => {
    const [n] = markdownToNodes(schema, '- Milch\n- Brot')
    expect(n!.type.name).toBe('bulletList')
    expect(n!.childCount).toBe(2)
    expect(n!.child(0).textContent).toBe('Milch')
  })

  it('falls back to a paragraph rather than dropping something it cannot parse', () => {
    // A proposal that silently loses a line is worse than one that renders it
    // plainly: only the second is visible to the person deciding.
    const [n] = markdownToNodes(schema, '|a|b|\n|-|-|')
    expect(n!.type.name).toBe('paragraph')
    expect(n!.textContent).toContain('|a|b|')
  })

  it('never returns nothing, so `replace` cannot become a silent delete', () => {
    expect(markdownToNodes(schema, '   ')).toHaveLength(1)
  })
})

describe('applyOps', () => {
  it('replaces only the block it names', () => {
    const state = stateWith(para('blk_a', 'First.'), para('blk_b', 'Second.'))
    const tr = state.tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'replace', id: 'blk_b', markdown: 'Second, tightened.' },
    ])
    expect(applied).toBe(1)
    expect(skipped).toEqual([])
    expect(tr.doc.child(0).textContent).toBe('First.')
    expect(tr.doc.child(1).textContent).toBe('Second, tightened.')
  })

  it('keeps later ops correct after an earlier one changed the length', () => {
    // The reason positions are re-resolved per op. A batch computed against the
    // original document lands increasingly wrong as it goes.
    const state = stateWith(para('blk_a', 'A'), para('blk_b', 'B'), para('blk_c', 'C'))
    const tr = state.tr
    const { applied } = applyOps(tr, schema, [
      { op: 'replace', id: 'blk_a', markdown: 'A much, much longer first paragraph.' },
      { op: 'replace', id: 'blk_c', markdown: 'C tightened.' },
    ])
    expect(applied).toBe(2)
    expect(tr.doc.child(1).textContent).toBe('B')
    expect(tr.doc.child(2).textContent).toBe('C tightened.')
  })

  it('inserts after the named block rather than before it', () => {
    const state = stateWith(para('blk_a', 'A'), para('blk_b', 'B'))
    const tr = state.tr
    applyOps(tr, schema, [{ op: 'insert_after', id: 'blk_a', markdown: 'A2' }])
    expect(tr.doc.childCount).toBe(3)
    expect(tr.doc.child(1).textContent).toBe('A2')
    expect(tr.doc.child(2).textContent).toBe('B')
  })

  it('skips an op whose block is gone and says so, rather than failing the batch', () => {
    // Somebody deleting a paragraph while an agent was thinking must not make an
    // otherwise good proposal unacceptable.
    const state = stateWith(para('blk_a', 'A'))
    const tr = state.tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'replace', id: 'blk_a', markdown: 'A tightened.' },
      { op: 'delete', id: 'blk_missing' },
    ])
    expect(applied).toBe(1)
    expect(skipped).toHaveLength(1)
    expect(skipped[0]).toContain('blk_missing')
    expect(tr.doc.child(0).textContent).toBe('A tightened.')
  })
})

describe('parseProposal', () => {
  it('returns null for anything that is not a proposal, without throwing', () => {
    expect(parseProposal('not json')).toBeNull()
    expect(parseProposal('{"id":"p"}')).toBeNull()
    expect(parseProposal(42)).toBeNull()
  })
})

describe('touchedBlocks', () => {
  it('counts only pending proposals — a decided one is a record, not a mark', () => {
    const p = (id: string, status: Proposal['status'], block: string): Proposal => ({
      id,
      status,
      author: 'agent:w1',
      instruction: '',
      summary: '',
      created_at: 1,
      skipped: [],
      ops: [{ op: 'replace', id: block, markdown: 'x' }],
    })
    const ids = touchedBlocks([p('p1', 'pending', 'blk_a'), p('p2', 'accepted', 'blk_b')])
    expect([...ids]).toEqual(['blk_a'])
  })
})

describe('a batch of ops lands the way it was written', () => {
  it('keeps two insertions after one block in their order', () => {
    const tr = stateWith(para('blk_one', 'Original.')).tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'insert_after', id: 'blk_one', markdown: 'First addition.' },
      { op: 'insert_after', id: 'blk_one', markdown: 'Second addition.' },
    ])
    expect(applied).toBe(2)
    expect(skipped).toEqual([])
    // Both re-found the same anchor, so the second used to land ABOVE the
    // first: a reviewer accepted an ordered list and got it backwards, with
    // nothing skipped to hint at it.
    expect(texts(tr.doc)).toEqual(['Original.', 'First addition.', 'Second addition.'])
  })

  it('lets a later op still address a block an earlier op replaced', () => {
    const tr = stateWith(para('blk_one', 'Original.')).tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'replace', id: 'blk_one', markdown: 'Rewritten.' },
      { op: 'insert_after', id: 'blk_one', markdown: 'Added after.' },
    ])
    // The replacement carries the id forward. Without that the insert was
    // dropped and reported as "that block is no longer in the document" —
    // blaming a peer for a removal this very accept had just performed.
    expect(skipped).toEqual([])
    expect(applied).toBe(2)
    expect(texts(tr.doc)).toEqual(['Rewritten.', 'Added after.'])
  })
})

describe('a batch whose ops change the shape under each other', () => {
  it('keeps the order when a replace changes the anchor size mid-batch', () => {
    const tr = stateWith(para('blk_a', 'AAAA'), para('blk_t', 'Tail.')).tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'insert_after', id: 'blk_a', markdown: 'X' },
      { op: 'replace', id: 'blk_a', markdown: 'PP\n\nQQ' },
      { op: 'insert_after', id: 'blk_a', markdown: 'Y' },
    ])
    // Named for what it actually pins. The byte-offset version fails this on
    // ORDER (`PP QQ Y X`), not by splitting anything — a reviewer checked, and
    // the first version of this test claimed the split in its name and comment
    // while proving something else. The split has its own case below.
    expect(applied).toBe(3)
    expect(skipped).toEqual([])
    expect(texts(tr.doc)).toEqual(['PP', 'QQ', 'X', 'Y', 'Tail.'])
  })

  it('inserts after the WHOLE of a multi-node replacement, not inside it', () => {
    const tr = stateWith(para('blk_a', 'AAAA'), para('blk_t', 'Tail.')).tr
    const { skipped } = applyOps(tr, schema, [
      { op: 'replace', id: 'blk_a', markdown: 'First half.\n\nSecond half.' },
      { op: 'insert_after', id: 'blk_a', markdown: 'Added after.' },
    ])
    // Only the first replacement node carries the id, so the anchor's own size
    // no longer covers what the block became.
    expect(skipped).toEqual([])
    expect(texts(tr.doc)).toEqual(['First half.', 'Second half.', 'Added after.', 'Tail.'])
  })

  it('forgets what trailed an id when that block is deleted', () => {
    // TWO blocks sharing an id, which `block-id.ts` calls the ordinary result of
    // a concurrent split. `findBlock` takes the first, so a count left over from
    // the deleted one is applied to the survivor and the insert walks past an
    // unrelated block.
    const tr = stateWith(
      para('blk_dup', 'one'),
      para('blk_dup', 'two'),
      para('blk_other', 'three'),
    ).tr
    applyOps(tr, schema, [
      { op: 'insert_after', id: 'blk_dup', markdown: 'X' },
      { op: 'delete', id: 'blk_dup' },
      { op: 'insert_after', id: 'blk_dup', markdown: 'Y' },
    ])
    expect(texts(tr.doc)).toEqual(['X', 'two', 'Y', 'three'])
  })
})

describe('the split the byte offset caused', () => {
  it('never cuts a block in half, whatever an earlier op did to sizes', () => {
    // THE corruption, on an input that actually produces it. Under the
    // byte-offset version this yields ["PP","QQQQQ","Y","Q","XXXX","Tail."] —
    // `QQQQQQ` cut in two — with `applied: 3` and `skipped: []`.
    //
    // Four earlier attempts in this file claimed to pin this and did not. Each
    // used a shape where the stale offset landed at the END of a block, where
    // ProseMirror's split leaves nothing behind, so no block came out empty and
    // the assertion never discriminated. This input puts it in the middle. The
    // input came from a reviewer, not from me — I had already convinced myself
    // three times.
    const tr = stateWith(para('blk_a', 'AAAA'), para('blk_t', 'Tail.')).tr
    const { applied, skipped } = applyOps(tr, schema, [
      { op: 'insert_after', id: 'blk_a', markdown: 'XXXX' },
      { op: 'replace', id: 'blk_a', markdown: 'PP\n\nQQQQQQ' },
      { op: 'insert_after', id: 'blk_a', markdown: 'Y' },
    ])
    expect(applied).toBe(3)
    expect(skipped).toEqual([])
    expect(texts(tr.doc)).toEqual(['PP', 'QQQQQQ', 'XXXX', 'Y', 'Tail.'])
  })
})
