import { describe, it, expect } from 'vitest'
import { epicOf, inSubtree, indexById, type TicketNode } from './tickets'

const tree = (nodes: TicketNode[]) => indexById(nodes)

describe('epicOf', () => {
  it('returns the top-most epic, so nested epics collapse into one group', () => {
    const idx = tree([
      { id: 'E1', type: 'epic' },
      { id: 'E2', type: 'epic', parent: 'E1' },
      { id: 'T1', type: 'task', parent: 'E2' },
    ])
    expect(epicOf(idx['T1'], idx)).toBe('E1')
    expect(epicOf(idx['E2'], idx)).toBe('E1')
    expect(epicOf(idx['E1'], idx)).toBe('E1')
  })

  it('returns "" for work under no epic', () => {
    const idx = tree([
      { id: 'T1', type: 'task' },
      { id: 'T2', type: 'task', parent: 'T1' },
    ])
    expect(epicOf(idx['T2'], idx)).toBe('')
  })

  it('tolerates a dangling parent', () => {
    const idx = tree([{ id: 'T1', type: 'task', parent: 'GONE' }])
    expect(epicOf(idx['T1'], idx)).toBe('')
  })

  it('terminates on a cycle instead of hanging', () => {
    const idx = tree([
      { id: 'A', type: 'task', parent: 'B' },
      { id: 'B', type: 'epic', parent: 'A' },
    ])
    expect(epicOf(idx['A'], idx)).toBe('B')
  })

  it('stops at the depth cap', () => {
    const nodes: TicketNode[] = [{ id: 'E', type: 'epic' }]
    for (let i = 0; i < 100; i++) {
      nodes.push({ id: 't' + i, type: 'task', parent: i === 0 ? 'E' : 't' + (i - 1) })
    }
    const idx = tree(nodes)
    // Deeper than MAX_DEPTH from the leaf, so the epic is out of reach — the
    // walk must return, not climb forever.
    expect(epicOf(idx['t99'], idx)).toBe('')
  })
})

describe('inSubtree', () => {
  it('matches the ticket itself and any descendant', () => {
    const idx = tree([
      { id: 'E1', type: 'epic' },
      { id: 'T1', type: 'task', parent: 'E1' },
      { id: 'S1', type: 'task', parent: 'T1' },
      { id: 'X', type: 'task' },
    ])
    expect(inSubtree(idx['E1'], 'E1', idx)).toBe(true)
    expect(inSubtree(idx['S1'], 'E1', idx)).toBe(true)
    expect(inSubtree(idx['X'], 'E1', idx)).toBe(false)
  })

  it('terminates on a cycle', () => {
    const idx = tree([
      { id: 'A', parent: 'B' },
      { id: 'B', parent: 'A' },
    ])
    expect(inSubtree(idx['A'], 'NOPE', idx)).toBe(false)
  })
})
