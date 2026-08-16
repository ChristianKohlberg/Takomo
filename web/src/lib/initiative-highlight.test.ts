import { describe, expect, it } from 'vitest'
import { decorate, topSpan, type Span } from './initiative-highlight'
import type { Run } from './initiative-doc'
import type { Entry } from './initiatives'

const src: Entry = {
  id: 'src-1',
  initiative: 'ini-1',
  kind: 'research',
  source: 'agent:w1',
  created_at: '2026-08-01T00:00:00.000Z',
  author: 'agent:w1',
}

function span(id: string, start: number, end: number, over: Partial<Span> = {}): Span {
  return { id, kind: 'thread', start, end, ...over }
}

/** Text of each piece, marks written as their source form. */
function shape(pieces: ReturnType<typeof decorate>): string[] {
  return pieces.map((p) => (p.kind === 'text' ? p.text : `[${p.cite}]`))
}

describe('decorate', () => {
  const runs: Run[] = [{ text: 'abcdefgh' }]

  it('returns the runs untouched when nothing is highlighted', () => {
    expect(decorate(runs, [])).toEqual([{ kind: 'text', text: 'abcdefgh', spans: [] }])
  })

  it('cuts a run at the span boundaries', () => {
    const pieces = decorate(runs, [span('t1', 2, 5)])
    expect(shape(pieces)).toEqual(['ab', 'cde', 'fgh'])
    expect(pieces.map((p) => p.spans.map((s) => s.id))).toEqual([[], ['t1'], []])
  })

  it('labels an overlapped piece with both spans', () => {
    const pieces = decorate(runs, [span('t1', 0, 5), span('t2', 3, 8)])
    expect(shape(pieces)).toEqual(['abc', 'de', 'fgh'])
    expect(pieces.map((p) => p.spans.map((s) => s.id))).toEqual([['t1'], ['t1', 't2'], ['t2']])
  })

  it('keeps a citation mark atomic rather than cutting it in half', () => {
    const withMark: Run[] = [{ text: 'ab' }, { cite: 1, entry: src }, { text: 'cd' }]
    // The mark occupies offsets 2..5 ("[1]"); this span ends inside it.
    const pieces = decorate(withMark, [span('t1', 0, 4)])
    expect(shape(pieces)).toEqual(['ab', '[1]', 'cd'])
    expect(pieces[1]?.spans.map((s) => s.id)).toEqual(['t1'])
    expect(pieces[2]?.spans).toEqual([])
  })

  it('leaves a mark outside every span unhighlighted', () => {
    const withMark: Run[] = [{ text: 'ab' }, { cite: 1, entry: src }]
    expect(decorate(withMark, [span('t1', 0, 2)])[1]?.spans).toEqual([])
  })

  it('ignores a zero-width span rather than emitting an empty piece', () => {
    const pieces = decorate(runs, [span('t1', 3, 3)])
    expect(shape(pieces)).toEqual(['abcdefgh'])
    expect(pieces[0]?.spans).toEqual([])
  })

  it('handles a span covering the whole paragraph', () => {
    const pieces = decorate(runs, [span('t1', 0, 8)])
    expect(shape(pieces)).toEqual(['abcdefgh'])
    expect(pieces[0]?.spans.map((s) => s.id)).toEqual(['t1'])
  })

  it('clips a span that runs past the end of the prose', () => {
    const pieces = decorate(runs, [span('t1', 6, 99)])
    expect(shape(pieces)).toEqual(['abcdef', 'gh'])
    expect(pieces[1]?.spans.map((s) => s.id)).toEqual(['t1'])
  })

  it('spans several runs', () => {
    const many: Run[] = [{ text: 'abc' }, { text: 'def' }]
    const pieces = decorate(many, [span('t1', 2, 4)])
    expect(shape(pieces)).toEqual(['ab', 'c', 'd', 'ef'])
    expect(pieces.map((p) => p.spans.length)).toEqual([0, 1, 1, 0])
  })
})

describe('topSpan', () => {
  it('picks the narrowest span, so an inner highlight wins its own words', () => {
    expect(topSpan([span('wide', 0, 20), span('tight', 5, 8)])?.id).toBe('tight')
  })

  it('is null when nothing covers the piece', () => {
    expect(topSpan([])).toBeNull()
  })
})
