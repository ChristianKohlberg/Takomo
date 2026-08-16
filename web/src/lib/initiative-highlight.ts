// Turning resolved anchors into something renderable.
//
// A paragraph is a list of runs (text, and citation marks that are atomic). A
// note or a suggestion covers a plain-text range that pays no attention to those
// boundaries — it may start mid-run, end mid-run, and swallow a mark on the way.
// So rendering a highlight means re-cutting the runs at every span boundary and
// asking, for each resulting piece, which spans cover it.
//
// Overlap is expected rather than exceptional: two readers highlight overlapping
// sentences all the time, and a piece covered by both must be able to say so —
// which is why a piece carries a LIST of spans rather than one.

import type { Entry } from './initiatives'
import type { Run } from './initiative-doc'

export type SpanKind = 'thread' | 'suggestion'

export interface Span {
  /** The entry id of the note or suggestion this span came from. */
  id: string
  kind: SpanKind
  start: number
  end: number
  /** Thread state, so an unresolved note can look different from a settled one. */
  state?: string
}

export type Piece =
  | { kind: 'text'; text: string; spans: Span[] }
  | { kind: 'cite'; cite: number; entry: Entry; spans: Span[] }

/** The plain length a run contributes — a mark counts as its source form, `[n]`. */
function runLength(r: Run): number {
  return ('text' in r ? r.text : `[${r.cite}]`).length
}

/**
 * Cut a paragraph's runs at every span boundary and label each piece with the
 * spans covering it.
 *
 * A zero-width span is dropped rather than rendered: there is nothing to
 * highlight, and an empty piece would render as an invisible click target.
 */
export function decorate(runs: Run[], spans: Span[]): Piece[] {
  const live = spans.filter((s) => s.end > s.start)
  if (live.length === 0) {
    return runs.map((r) =>
      'text' in r
        ? { kind: 'text' as const, text: r.text, spans: [] }
        : { kind: 'cite' as const, cite: r.cite, entry: r.entry, spans: [] },
    )
  }

  const cuts = new Set<number>()
  for (const s of live) {
    cuts.add(s.start)
    cuts.add(s.end)
  }

  const covering = (from: number, to: number): Span[] =>
    live.filter((s) => s.start < to && s.end > from)

  const out: Piece[] = []
  let pos = 0
  for (const run of runs) {
    const len = runLength(run)
    const start = pos
    const end = pos + len
    pos = end

    if (!('text' in run)) {
      // A mark is atomic: it is either inside a span or it is not, and cutting
      // one in half would produce two half-citations that cite nothing.
      out.push({ kind: 'cite', cite: run.cite, entry: run.entry, spans: covering(start, end) })
      continue
    }

    // Boundaries strictly inside this run, in order.
    const inner = [...cuts].filter((c) => c > start && c < end).sort((a, b) => a - b)
    let at = start
    for (const cut of [...inner, end]) {
      const text = run.text.slice(at - start, cut - start)
      if (text) out.push({ kind: 'text', text, spans: covering(at, cut) })
      at = cut
    }
  }
  return out
}

/**
 * Which single span a piece should be attributed to when it is clicked.
 *
 * The narrowest one wins. A reader who highlighted a phrase inside someone
 * else's highlighted paragraph means the phrase — the enclosing span is still
 * reachable by clicking the part of it that is not overlapped.
 */
export function topSpan(spans: Span[]): Span | null {
  let best: Span | null = null
  for (const s of spans) {
    if (!best || s.end - s.start < best.end - best.start) best = s
  }
  return best
}
