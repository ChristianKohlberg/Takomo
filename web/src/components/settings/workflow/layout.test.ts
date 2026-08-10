import { describe, expect, it } from 'vitest'
import { autoLayout, edgeAnchors, layoutExtent, rankStates, NODE_W } from './layout'
import type { WorkflowDoc } from '@/lib/workflows'

const simple: WorkflowDoc = {
  name: 'simple',
  initial: 'todo',
  states: [
    { id: 'todo', category: 'todo', claimable: true },
    { id: 'in_progress', category: 'in_progress' },
    { id: 'blocked', category: 'blocked' },
    { id: 'done', category: 'done', terminal: true },
    { id: 'cancelled', category: 'cancelled', terminal: true },
  ],
  transitions: [
    { from: 'todo', to: 'in_progress', requires: ['claim'] },
    { from: 'todo', to: 'cancelled' },
    { from: 'in_progress', to: 'done', requires: ['claim'] },
    { from: 'in_progress', to: 'blocked' },
    { from: 'blocked', to: 'in_progress' },
  ],
}

describe('rankStates', () => {
  it('ranks by shortest distance from the initial state', () => {
    const r = rankStates(simple)
    expect(r.get('todo')).toBe(0)
    expect(r.get('in_progress')).toBe(1)
    expect(r.get('cancelled')).toBe(1)
    expect(r.get('done')).toBe(2)
    expect(r.get('blocked')).toBe(2)
  })

  // An unreachable state is a defect the validator reports — and the editor is
  // exactly where someone goes to fix it, so it must still be placed and
  // draggable rather than dropped from the picture.
  it('still places a state nothing transitions into', () => {
    const orphaned: WorkflowDoc = {
      ...simple,
      states: [...simple.states, { id: 'limbo', category: 'todo' }],
    }
    const r = rankStates(orphaned)
    expect(r.has('limbo')).toBe(true)
    expect(r.get('limbo')).toBeGreaterThan(0)
  })

  it('does not loop forever on a cycle', () => {
    const cyclic: WorkflowDoc = {
      name: 'c',
      initial: 'a',
      states: [
        { id: 'a', category: 'todo' },
        { id: 'b', category: 'in_progress' },
        { id: 'end', category: 'done', terminal: true },
      ],
      transitions: [
        { from: 'a', to: 'b' },
        { from: 'b', to: 'a' },
        { from: 'b', to: 'end' },
      ],
    }
    const r = rankStates(cyclic)
    expect(r.get('a')).toBe(0)
    expect(r.get('b')).toBe(1)
    expect(r.get('end')).toBe(2)
  })
})

describe('autoLayout', () => {
  it('is deterministic — the same document lays out identically', () => {
    expect(autoLayout(simple)).toEqual(autoLayout(simple))
  })

  it('places every state exactly once, with no two sharing a position', () => {
    const l = autoLayout(simple)
    expect(Object.keys(l).sort()).toEqual(simple.states.map((s) => s.id).sort())
    const seen = new Set(Object.values(l).map((p) => `${p.x},${p.y}`))
    expect(seen.size).toBe(simple.states.length)
  })

  it('runs the lifecycle left to right', () => {
    const l = autoLayout(simple)
    expect(l.todo!.x).toBeLessThan(l.in_progress!.x)
    expect(l.in_progress!.x).toBeLessThan(l.done!.x)
  })

  it('sinks endings below live work in the same column', () => {
    const l = autoLayout(simple)
    // Both are one hop from `todo`.
    expect(l.cancelled!.y).toBeGreaterThan(l.in_progress!.y)
  })

  // Dragging one box must not forfeit the automatic placement of the others,
  // and a state added afterwards must not land at the origin under another node.
  it('honours stored positions per node and auto-places the rest', () => {
    const l = autoLayout(simple, { done: { x: 999, y: 111 } })
    expect(l.done).toEqual({ x: 999, y: 111 })
    expect(l.todo).not.toEqual({ x: 999, y: 111 })
    expect(Object.keys(l)).toHaveLength(simple.states.length)
  })

  it('ignores a stored position for a state that no longer exists', () => {
    const l = autoLayout(simple, { ghost: { x: 5, y: 5 } })
    expect(l.ghost).toBeUndefined()
  })
})

describe('layoutExtent', () => {
  it('covers the furthest node plus its own width', () => {
    const { width, height } = layoutExtent({ a: { x: 100, y: 50 } })
    expect(width).toBeGreaterThan(100 + NODE_W - 1)
    expect(height).toBeGreaterThan(50)
  })
})

describe('edgeAnchors', () => {
  it('leaves the right side going forwards', () => {
    const a = edgeAnchors({ x: 0, y: 0 }, { x: 300, y: 0 })
    expect(a.x1).toBe(NODE_W)
    expect(a.x2).toBe(300)
  })

  // review -> implementing is the backwards edge every real workflow has. It
  // must leave from the LEFT, or it crosses back through its own source box.
  it('leaves the left side going backwards', () => {
    const a = edgeAnchors({ x: 300, y: 0 }, { x: 0, y: 0 })
    expect(a.x1).toBe(300)
    expect(a.x2).toBe(NODE_W)
  })
})
