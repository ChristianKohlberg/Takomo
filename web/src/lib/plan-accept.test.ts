// The accept path's RECORD, which is the half that survives a reload.
//
// Three of the five browser fixes shipped with no test at all, and the one that
// mattered — "nothing applied is not an acceptance" — was fixed on the plan view
// and missed on the standalone editor. A test here would have caught that; it is
// the reason this file exists.
import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { decideProposal, readProposals, PROPOSALS_KEY } from './plan-proposals'

function mapWith(id: string): Y.Map<unknown> {
  const doc = new Y.Doc()
  const map = doc.getMap(PROPOSALS_KEY)
  map.set(
    id,
    JSON.stringify({
      id,
      status: 'pending',
      node: 'mn-1',
      instruction: 'i',
      summary: 's',
      ops: [{ op: 'replace', id: 'blk_one', markdown: 'x' }],
      created_at: 1,
    }),
  )
  return map as Y.Map<unknown>
}

describe('recording a decision', () => {
  it('writes what the accept could not apply, so a reload can still see it', () => {
    const map = mapWith('prop-a')
    expect(
      decideProposal(map as never, 'prop-a', 'accepted', 'Ada', 2, [
        'replace blk_gone: that block is no longer in the document',
      ]),
    ).toBe(true)
    const [p] = readProposals(map as never)
    expect(p!.status).toBe('accepted')
    // Without this the only sign a part of an "accepted" proposal was dropped
    // was a toast, which does not survive a reload.
    expect((p as unknown as { dropped: string[] }).dropped).toEqual([
      'replace blk_gone: that block is no longer in the document',
    ])
  })

  it('leaves nothing behind when the proposal is already decided', () => {
    const map = mapWith('prop-a')
    expect(decideProposal(map as never, 'prop-a', 'accepted', 'Ada')).toBe(true)
    // A second reviewer whose peer has seen the first decision is refused. This
    // is NOT consensus across peers that have not synced — see the function's
    // own comment; it catches a double click and a peer that is up to date.
    expect(decideProposal(map as never, 'prop-a', 'rejected', 'Sam')).toBe(false)
    const [p] = readProposals(map as never)
    expect(p!.status).toBe('accepted')
    expect(p!.decided_by).toBe('Ada')
  })

  it('records no `dropped` key when the whole change landed', () => {
    const map = mapWith('prop-a')
    decideProposal(map as never, 'prop-a', 'accepted', 'Ada', 2, [])
    const [p] = readProposals(map as never)
    expect('dropped' in (p as object)).toBe(false)
  })
})
