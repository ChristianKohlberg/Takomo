// What an agent offered, read section by section — and what accepting one
// actually does.
//
// The two halves of a decision are tested TOGETHER here, because separately
// they each look fine and the bug lives between them: applying ops to the wrong
// section, or marking a record accepted without the edit landing, is what turns
// "an agent proposes, a person confirms" into a claim nobody can check.
import { describe, expect, it } from 'vitest'
import { Schema } from '@tiptap/pm/model'
import { EditorState } from '@tiptap/pm/state'
import * as Y from 'yjs'

import { applyOps, type Proposal } from './doc-ops'
import { planSections } from './plan-sections'
import {
  byUndecidedThenNewest,
  decideProposal,
  highlightKeyFor,
  pendingByNode,
  pendingInSubtree,
  proposalsByNode,
  readProposals,
  PROPOSALS_KEY,
} from './plan-proposals'

function proposal(over: Partial<Proposal> & { id: string }): Proposal {
  return {
    status: 'pending',
    node: 'mn-1',
    author: 'agent:fleet-3',
    instruction: '',
    summary: '',
    created_at: 1,
    skipped: [],
    ops: [],
    ...over,
  }
}

/** A map shaped the way the server writes one: JSON per entry. */
function docWith(...list: Proposal[]): { doc: Y.Doc; map: Y.Map<string> } {
  const doc = new Y.Doc()
  const map = doc.getMap<string>(PROPOSALS_KEY)
  for (const p of list) map.set(p.id, JSON.stringify(p))
  return { doc, map }
}

describe('readProposals', () => {
  it('reads the records and drops one it cannot parse, rather than blanking', () => {
    const { map } = docWith(proposal({ id: 'prop-a' }))
    map.set('prop-broken', 'not json at all')
    expect(readProposals(map).map((p) => p.id)).toEqual(['prop-a'])
  })
})

describe('proposalsByNode', () => {
  it('files each proposal under the section it names, and nowhere else', () => {
    const grouped = proposalsByNode([
      proposal({ id: 'prop-a', node: 'mn-1' }),
      proposal({ id: 'prop-b', node: 'mn-2' }),
      proposal({ id: 'prop-c', node: 'mn-1' }),
    ])
    expect(grouped.get('mn-1')?.map((p) => p.id).sort()).toEqual(['prop-a', 'prop-c'])
    expect(grouped.get('mn-2')?.map((p) => p.id)).toEqual(['prop-b'])
  })

  it('leaves a proposal about no section out of every group', () => {
    // That is what a standalone document's proposals look like. One arriving
    // here is about a document this view is not showing.
    const grouped = proposalsByNode([proposal({ id: 'prop-a', node: null })])
    expect(grouped.size).toBe(0)
  })

  it('puts undecided first, newest first within that', () => {
    const grouped = proposalsByNode([
      proposal({ id: 'old', created_at: 1 }),
      proposal({ id: 'decided', status: 'rejected', created_at: 9 }),
      proposal({ id: 'new', created_at: 5 }),
    ])
    expect(grouped.get('mn-1')?.map((p) => p.id)).toEqual(['new', 'old', 'decided'])
  })

  it('keeps a rejected proposal, because a decision is recorded and not erased', () => {
    const grouped = proposalsByNode([proposal({ id: 'prop-a', status: 'rejected' })])
    expect(grouped.get('mn-1')).toHaveLength(1)
  })
})

describe('byUndecidedThenNewest', () => {
  it('does not reorder two proposals of the same standing by anything but age', () => {
    const a = proposal({ id: 'a', created_at: 2 })
    const b = proposal({ id: 'b', created_at: 3 })
    expect([a, b].sort(byUndecidedThenNewest).map((p) => p.id)).toEqual(['b', 'a'])
  })
})

describe('pendingByNode', () => {
  it('counts only what is still waiting on a person', () => {
    const counts = pendingByNode([
      proposal({ id: 'a', node: 'mn-1' }),
      proposal({ id: 'b', node: 'mn-1' }),
      proposal({ id: 'c', node: 'mn-1', status: 'accepted' }),
      proposal({ id: 'd', node: 'mn-2', status: 'rejected' }),
    ])
    expect(counts).toEqual({ 'mn-1': 2 })
  })
})

describe('pendingInSubtree', () => {
  const sections = planSections([
    { id: 'a', parent: null, order: 'a0', title: 'Payments', position: 0 },
    { id: 'b', parent: 'a', order: 'a0', title: 'API', position: 0 },
    { id: 'c', parent: 'b', order: 'a0', title: 'Versioning', position: 0 },
  ])

  it('reports what is waiting anywhere beneath a folded section', () => {
    // A reader who folded a branch has not decided they do not care what an
    // agent offered inside it.
    expect(pendingInSubtree(sections[0]!, { c: 2 })).toBe(2)
    expect(pendingInSubtree(sections[0]!, { a: 1, c: 2 })).toBe(3)
  })

  it('is zero when nothing under it is waiting', () => {
    expect(pendingInSubtree(sections[0]!, {})).toBe(0)
  })
})

describe('highlightKeyFor', () => {
  it('names the blocks pending proposals touch, and only those', () => {
    const key = highlightKeyFor([
      proposal({ id: 'a', ops: [{ op: 'replace', id: 'blk_b', markdown: 'x' }] }),
      proposal({
        id: 'b',
        status: 'accepted',
        ops: [{ op: 'replace', id: 'blk_z', markdown: 'x' }],
      }),
    ])
    expect(key).toBe('blk_b')
  })

  it('is stable, so the editor is not asked to redraw when nothing moved', () => {
    const ops = [
      { op: 'replace' as const, id: 'blk_b', markdown: 'x' },
      { op: 'delete' as const, id: 'blk_a' },
    ]
    expect(highlightKeyFor([proposal({ id: 'a', ops })])).toBe('blk_a blk_b')
    expect(highlightKeyFor([proposal({ id: 'a', ops: [...ops].reverse() })])).toBe(
      'blk_a blk_b',
    )
  })
})

describe('decideProposal', () => {
  it('records who decided, and keeps the proposal readable afterwards', () => {
    const { map } = docWith(proposal({ id: 'prop-a', summary: 'Tighten it.' }))
    expect(decideProposal(map, 'prop-a', 'rejected', 'Ada', 1700)).toBe(true)
    const after = readProposals(map)[0]!
    expect(after.status).toBe('rejected')
    expect(after.decided_by).toBe('Ada')
    expect(after.decided_at).toBe(1700)
    expect(after.summary).toBe('Tighten it.')
  })

  it('refuses a second decision, so two reviewers cannot both apply it', () => {
    const { map } = docWith(proposal({ id: 'prop-a' }))
    expect(decideProposal(map, 'prop-a', 'accepted', 'Ada')).toBe(true)
    expect(decideProposal(map, 'prop-a', 'accepted', 'Ben')).toBe(false)
  })

  it('refuses a proposal that is not there', () => {
    const { map } = docWith()
    expect(decideProposal(map, 'prop-gone', 'accepted', 'Ada')).toBe(false)
  })
})

// ---- the decision, end to end ---------------------------------------------

// Close enough to StarterKit for these to mean something: the node names and the
// `id` attribute are what the ops address. Same shape as `doc-ops.test.ts`.
const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    text: { group: 'inline' },
    paragraph: { group: 'block', content: 'inline*', attrs: { id: { default: null } } },
  },
})

const para = (id: string, text: string) => schema.nodes.paragraph!.create({ id }, schema.text(text))

const stateWith = (...nodes: ReturnType<typeof para>[]) =>
  EditorState.create({ schema, doc: schema.nodes.doc!.create(null, nodes) })

describe('accepting and rejecting', () => {
  it('applies exactly the named ops and leaves the rest of the section alone', () => {
    const state = stateWith(
      para('blk_a', 'The surface everything hangs off.'),
      para('blk_b', 'Billing.'),
      para('blk_c', 'Untouched.'),
    )
    const p = proposal({
      id: 'prop-a',
      ops: [{ op: 'replace', id: 'blk_b', markdown: 'Billing, spelled out.' }],
    })
    const { map } = docWith(p)

    const tr = state.tr
    const { applied, skipped } = applyOps(tr, schema, p.ops)
    expect(decideProposal(map, p.id, 'accepted', 'Ada')).toBe(true)

    expect(applied).toBe(1)
    expect(skipped).toEqual([])
    expect(tr.doc.child(0).textContent).toBe('The surface everything hangs off.')
    expect(tr.doc.child(1).textContent).toBe('Billing, spelled out.')
    expect(tr.doc.child(2).textContent).toBe('Untouched.')
    expect(readProposals(map)[0]!.status).toBe('accepted')
  })

  it('reports an op whose block has gone rather than dropping it silently', () => {
    const state = stateWith(para('blk_a', 'A.'))
    const p = proposal({
      id: 'prop-a',
      ops: [
        { op: 'replace', id: 'blk_a', markdown: 'A, tightened.' },
        { op: 'delete', id: 'blk_gone' },
      ],
    })
    const tr = state.tr
    const { applied, skipped } = applyOps(tr, schema, p.ops)
    expect(applied).toBe(1)
    expect(skipped).toHaveLength(1)
    expect(skipped[0]).toContain('blk_gone')
  })

  it('changes no prose when a proposal is rejected', () => {
    const state = stateWith(para('blk_a', 'As it stands.'))
    const p = proposal({
      id: 'prop-a',
      ops: [{ op: 'replace', id: 'blk_a', markdown: 'Rewritten.' }],
    })
    const { map } = docWith(p)

    expect(decideProposal(map, p.id, 'rejected', 'Ada')).toBe(true)

    expect(state.doc.child(0).textContent).toBe('As it stands.')
    const after = readProposals(map)[0]!
    expect(after.status).toBe('rejected')
    // Still there, still readable: "we considered this and said no" is the thing
    // somebody wants three weeks later when it is proposed again.
    expect(after.ops).toHaveLength(1)
  })
})
