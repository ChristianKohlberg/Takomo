import { describe, expect, it } from 'vitest'

import { buildDoc, threadsFor, type Pane } from './initiative-doc'
import type { Entry } from './initiatives'

let seq = 0

function entry(kind: string, over: Partial<Entry> = {}): Entry {
  seq += 1
  return {
    id: over.id ?? `ent-${seq}`,
    initiative: 'ini-1',
    kind,
    source: 'agent:w1',
    text: '',
    created_at: `2026-08-0${Math.min(seq, 9)}T00:00:00.000Z`,
    author: 'agent:w1',
    ...over,
  }
}

function view(pane: Pane, text: string, cites: string[] = [], over: Partial<Entry> = {}): Entry {
  return entry('view', { text, meta: { pane, cites }, ...over })
}

describe('buildDoc', () => {
  it('reports no document when nothing has been written', () => {
    const doc = buildDoc([entry('note', { text: 'just a note' })])
    expect(doc.hasDocument).toBe(false)
    expect(doc.panes.business.paragraphs).toEqual([])
    expect(doc.sources).toEqual([])
  })

  it('splits prose into paragraphs on blank lines', () => {
    const doc = buildDoc([view('business', 'First para.\n\nSecond para.\n\n\nThird.')])
    expect(doc.panes.business.paragraphs).toHaveLength(3)
    expect(doc.panes.business.paragraphs[0]?.runs).toEqual([{ text: 'First para.' }])
  })

  it('resolves a citation mark to the entry its pane cites', () => {
    const src = entry('transcript', { id: 'ent-src', text: 'the call' })
    const doc = buildDoc([src, view('business', 'They re-key it[1] by hand.', ['ent-src'])])

    const runs = doc.panes.business.paragraphs[0]?.runs
    expect(runs).toEqual([
      { text: 'They re-key it' },
      { cite: 1, entry: src },
      { text: ' by hand.' },
    ])
    expect(doc.panes.business.paragraphs[0]?.uncited).toBe(false)
    expect(doc.sources).toEqual([src])
  })

  it('numbers a source once, globally, even when two panes cite it', () => {
    const a = entry('transcript', { id: 'ent-a' })
    const b = entry('code-research', { id: 'ent-b' })
    const doc = buildDoc([
      a,
      b,
      view('business', 'Business cites[1].', ['ent-a']),
      // Local index 1 here is a DIFFERENT entry, and local 2 is the shared one.
      view('technical', 'Technical cites[1] and[2].', ['ent-b', 'ent-a']),
    ])

    expect(doc.sources.map((s) => s.id)).toEqual(['ent-a', 'ent-b'])
    const tech = doc.panes.technical.paragraphs[0]?.runs.filter((r) => 'cite' in r)
    // b is source 2, a is source 1 — the reader's numbering, not the author's.
    expect(tech).toEqual([
      { cite: 2, entry: b },
      { cite: 1, entry: a },
    ])
  })

  it('flags a paragraph nobody sourced', () => {
    const src = entry('note', { id: 'ent-src' })
    const doc = buildDoc([src, view('business', 'Sourced[1].\n\nSomebody just said so.', ['ent-src'])])
    expect(doc.panes.business.paragraphs.map((p) => p.uncited)).toEqual([false, true])
  })

  it('leaves a stale citation as literal text rather than dropping it', () => {
    // `[2]` indexes past the end of `cites`, and `[1]` points at an entry that
    // is not on this page. Both must stay visible: a broken citation that
    // silently vanishes reads as an uncited sentence, which is a lie.
    const doc = buildDoc([view('business', 'Gone[1] and missing[2].', ['ent-not-here'])])
    const runs = doc.panes.business.paragraphs[0]?.runs
    expect(runs).toEqual([{ text: 'Gone[1] and missing[2].' }])
    expect(doc.sources).toEqual([])
  })

  it('takes the newest view per pane, so a revision supersedes', () => {
    const old = view('business', 'The old position.', [], {
      id: 'ent-old',
      created_at: '2026-08-01T00:00:00.000Z',
    })
    const next = view('business', 'The revised position.', [], {
      id: 'ent-new',
      created_at: '2026-08-09T00:00:00.000Z',
    })
    // Deliberately oldest-first: the reducer must not trust input ordering.
    const doc = buildDoc([old, next])
    expect(doc.panes.business.entry?.id).toBe('ent-new')
    expect(doc.panes.business.paragraphs[0]?.runs).toEqual([{ text: 'The revised position.' }])
  })

  it('breaks a same-millisecond tie deterministically', () => {
    const at = '2026-08-05T00:00:00.000Z'
    const a = view('business', 'A', [], { id: 'ent-a', created_at: at })
    const b = view('business', 'B', [], { id: 'ent-b', created_at: at })
    expect(buildDoc([a, b]).panes.business.entry?.id).toBe('ent-b')
    expect(buildDoc([b, a]).panes.business.entry?.id).toBe('ent-b')
  })

  it('anchors threads to their paragraph and clamps one that outlived it', () => {
    const doc = buildDoc([
      view('technical', 'One.\n\nTwo.'),
      entry('thread', { id: 'ent-t1', text: 'beside two', meta: { pane: 'technical', para: 1 } }),
      // Written against a paragraph that no longer exists.
      entry('thread', { id: 'ent-t9', text: 'orphan', meta: { pane: 'technical', para: 7 } }),
      entry('thread', { id: 'ent-tx', text: 'other pane', meta: { pane: 'business', para: 0 } }),
    ])

    const tech = doc.panes.technical
    expect(tech.threads).toHaveLength(2)
    expect(threadsFor(tech, 1).map((t) => t.entry.id)).toEqual(['ent-t1', 'ent-t9'])
    expect(threadsFor(tech, 0)).toEqual([])
    expect(doc.panes.business.threads).toHaveLength(1)
  })

  it('reads a thread state, defaulting anything unknown to open', () => {
    const doc = buildDoc([
      view('business', 'x'),
      entry('thread', { id: 'a', meta: { pane: 'business', para: 0, state: 'running' } }),
      entry('thread', { id: 'b', meta: { pane: 'business', para: 0, state: 'nonsense' } }),
      entry('thread', { id: 'c', meta: { pane: 'business', para: 0 } }),
    ])
    expect(doc.panes.business.threads.map((t) => t.state)).toEqual(['running', 'open', 'open'])
  })

  it('ignores a view with no pane, or a pane nobody defined', () => {
    const doc = buildDoc([
      entry('view', { text: 'nowhere', meta: {} }),
      entry('view', { text: 'nowhere', meta: { pane: 'legal' } }),
    ])
    expect(doc.hasDocument).toBe(false)
  })

  it('survives meta that is not an object', () => {
    const doc = buildDoc([
      entry('view', { text: 'x', meta: 'not an object' }),
      entry('view', { text: 'y', meta: null }),
      entry('view', { text: 'z', meta: ['a'] }),
    ])
    expect(doc.hasDocument).toBe(false)
  })
})
