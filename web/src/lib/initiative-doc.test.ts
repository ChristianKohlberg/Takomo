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

describe('origins', () => {
  it('collects entries marked as how the idea arrived, oldest first', () => {
    const late = entry('transcript', {
      id: 'ent-late',
      meta: { origin: true },
      origin_at: '2026-06-28T00:00:00.000Z',
    })
    const early = entry('transcript', {
      id: 'ent-early',
      meta: { origin: true },
      origin_at: '2026-03-12T00:00:00.000Z',
    })
    const notOrigin = entry('note', { id: 'ent-plain' })
    // Newest-first in, oldest-first out: this is the beginning of the story.
    const doc = buildDoc([late, early, notOrigin])
    expect(doc.origins.map((o) => o.id)).toEqual(['ent-early', 'ent-late'])
  })

  it('falls back to created_at when an origin has no origin_at', () => {
    const a = entry('note', { id: 'a', meta: { origin: true }, created_at: '2026-08-02T00:00:00.000Z' })
    const b = entry('note', { id: 'b', meta: { origin: true }, created_at: '2026-08-01T00:00:00.000Z' })
    expect(buildDoc([a, b]).origins.map((o) => o.id)).toEqual(['b', 'a'])
  })
})

describe('threads that supersede', () => {
  it('replaces a note with the one that supersedes it', () => {
    const doc = buildDoc([
      view('business', 'One.'),
      entry('thread', { id: 'th-1', text: 'ask someone', meta: { pane: 'business', para: 0 } }),
      entry('thread', {
        id: 'th-2',
        text: 'ask someone',
        meta: { pane: 'business', para: 0, state: 'running', ticket: 'demo-9d3', supersedes: 'th-1' },
      }),
    ])
    const threads = doc.panes.business.threads
    expect(threads).toHaveLength(1)
    expect(threads[0]?.entry.id).toBe('th-2')
    expect(threads[0]?.state).toBe('running')
    expect(threads[0]?.ticket).toBe('demo-9d3')
  })

  it('reports no ticket when the note has not been dispatched', () => {
    const doc = buildDoc([
      view('business', 'One.'),
      entry('thread', { id: 'th', meta: { pane: 'business', para: 0 } }),
    ])
    expect(doc.panes.business.threads[0]?.ticket).toBeNull()
  })
})

describe('amendments', () => {
  it('keeps a proposed view out of the live pane and offers it as pending', () => {
    const doc = buildDoc([
      view('business', 'The live position.'),
      view('business', 'The proposed position.', [], { id: 'prop-1', meta: { pane: 'business', cites: [], proposed: true } }),
    ])
    // The live pane is untouched until somebody accepts.
    expect(doc.panes.business.paragraphs[0]?.runs).toEqual([{ text: 'The live position.' }])
    expect(doc.panes.business.pending?.entry.id).toBe('prop-1')
    expect(doc.panes.business.pending?.paragraphs[0]?.runs).toEqual([
      { text: 'The proposed position.' },
    ])
  })

  it('diffs the proposal against the live pane, paragraph by paragraph', () => {
    const doc = buildDoc([
      view('business', 'Kept.\n\nOld wording.\n\nDropped.'),
      view('business', 'Kept.\n\nNew wording.', [], {
        id: 'prop-1',
        meta: { pane: 'business', cites: [], proposed: true },
      }),
    ])
    expect(doc.panes.business.pending?.diff).toEqual([
      { kind: 'same', text: 'Kept.' },
      { kind: 'changed', text: 'New wording.', was: 'Old wording.' },
      { kind: 'removed', text: 'Dropped.' },
    ])
  })

  it('marks a paragraph the proposal adds', () => {
    const doc = buildDoc([
      view('business', 'One.'),
      view('business', 'One.\n\nTwo.', [], {
        id: 'prop-1',
        meta: { pane: 'business', cites: [], proposed: true },
      }),
    ])
    expect(doc.panes.business.pending?.diff[1]).toEqual({ kind: 'added', text: 'Two.' })
  })

  it('drops a proposal once it has been decided, either way', () => {
    const proposal = view('business', 'Proposed.', [], {
      id: 'prop-1',
      meta: { pane: 'business', cites: [], proposed: true },
    })
    const live = view('business', 'Live.')

    const rejected = buildDoc([
      live,
      proposal,
      entry('decision', { id: 'dec-1', meta: { rejects: 'prop-1' } }),
    ])
    expect(rejected.panes.business.pending).toBeNull()
    expect(rejected.panes.business.paragraphs[0]?.runs).toEqual([{ text: 'Live.' }])

    const accepted = buildDoc([
      live,
      proposal,
      entry('decision', { id: 'dec-2', meta: { accepts: 'prop-1' } }),
      // Accepting appends the proposed text as a real view — that is what makes
      // it live, not the decision entry itself.
      view('business', 'Proposed.', [], { id: 'v-new', created_at: '2026-08-09T00:00:00.000Z' }),
    ])
    expect(accepted.panes.business.pending).toBeNull()
    expect(accepted.panes.business.paragraphs[0]?.runs).toEqual([{ text: 'Proposed.' }])
  })

  it('numbers citations a proposal introduces, so its marks resolve before acceptance', () => {
    const src = entry('research', { id: 'ent-new-src' })
    const doc = buildDoc([
      src,
      view('business', 'Live, uncited.'),
      view('business', 'Now with a source[1].', ['ent-new-src'], {
        id: 'prop-1',
        meta: { pane: 'business', cites: ['ent-new-src'], proposed: true },
      }),
    ])
    const runs = doc.panes.business.pending?.paragraphs[0]?.runs
    expect(runs).toEqual([
      { text: 'Now with a source' },
      { cite: 1, entry: src },
      { text: '.' },
    ])
    expect(doc.sources.map((s) => s.id)).toEqual(['ent-new-src'])
  })

  it('offers a proposal for a pane nobody has written yet', () => {
    const doc = buildDoc([
      view('business', 'Live.'),
      view('technical', 'First draft.', [], {
        id: 'prop-t',
        meta: { pane: 'technical', cites: [], proposed: true },
      }),
    ])
    expect(doc.panes.technical.entry).toBeNull()
    expect(doc.panes.technical.pending?.diff).toEqual([{ kind: 'added', text: 'First draft.' }])
  })
})
