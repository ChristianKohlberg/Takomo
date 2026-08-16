// Range anchors: how a comment, a suggestion or a citation stays attached to the
// words it was written about, across revisions of the prose.
//
// The document is append-only, so a pane is revised by appending a whole new
// `view` — the text under an existing note can therefore change, move, or
// disappear entirely between two reads. The old model anchored a note to a
// paragraph INDEX and clamped it into range on read, which meant a note silently
// slid onto a paragraph it was never about and nothing could tell that it had.
//
// So an anchor records the words instead: the `quote` that was highlighted, plus
// a little `prefix` and `suffix` so the same sentence appearing twice is not
// ambiguous. Resolution searches for those words and reports HOW it found them:
//
//   exact      prefix + quote + suffix all still adjacent — certainly the same place
//   moved      the quote is still there, but its surroundings changed
//   orphaned   the words are gone; the note is still someone's unanswered point
//
// Orphaned is a first-class outcome, not a failure to hide. A note whose text was
// rewritten out from under it is exactly the note a reader most needs to see.
//
// Everything here is pure and offset-based, with one DOM helper at the bottom
// that converts a browser selection into plain-text offsets. That split is what
// keeps the interesting half unit-testable without a browser.

/** Characters of surrounding context stored on each side of the quote. */
export const CONTEXT = 32

export interface Anchor {
  /** Which pane's prose the quote came from. */
  pane: string
  /** The paragraph index at the time of writing — a hint for resolution, not the anchor. */
  para: number
  /** The highlighted words. This is the anchor. */
  quote: string
  /** Up to CONTEXT characters immediately before the quote. */
  prefix: string
  /** Up to CONTEXT characters immediately after the quote. */
  suffix: string
}

export type Placement = 'exact' | 'moved'

export interface Placed {
  para: number
  start: number
  end: number
  how: Placement
}

/** Build an anchor from a plain-text offset range within one paragraph. */
export function makeAnchor(
  paras: string[],
  pane: string,
  para: number,
  start: number,
  end: number,
): Anchor | null {
  const text = paras[para]
  if (text === undefined) return null
  const lo = Math.max(0, Math.min(start, end))
  const hi = Math.min(text.length, Math.max(start, end))
  const quote = text.slice(lo, hi)
  // A zero-width selection anchors nothing. Callers treat null as "nothing to
  // act on" rather than storing an anchor that can never resolve.
  if (!quote.trim()) return null
  return {
    pane,
    para,
    quote,
    prefix: text.slice(Math.max(0, lo - CONTEXT), lo),
    suffix: text.slice(hi, Math.min(text.length, hi + CONTEXT)),
  }
}

/** Read an anchor back off an entry's free-form `meta`, or null if it has none. */
export function anchorOf(meta: unknown): Anchor | null {
  if (!meta || typeof meta !== 'object') return null
  const m = meta as Record<string, unknown>
  if (typeof m.quote !== 'string' || !m.quote) return null
  return {
    pane: typeof m.pane === 'string' ? m.pane : '',
    para: typeof m.para === 'number' && Number.isInteger(m.para) ? m.para : 0,
    quote: m.quote,
    prefix: typeof m.prefix === 'string' ? m.prefix : '',
    suffix: typeof m.suffix === 'string' ? m.suffix : '',
  }
}

/** Paragraph indices to search, the hinted one first — a note usually has not moved far. */
function order(count: number, hint: number): number[] {
  const rest: number[] = []
  for (let i = 0; i < count; i += 1) if (i !== hint) rest.push(i)
  return hint >= 0 && hint < count ? [hint, ...rest] : rest
}

/** Every index at which `needle` occurs in `hay`. */
function occurrences(hay: string, needle: string): number[] {
  const out: number[] = []
  if (!needle) return out
  let at = hay.indexOf(needle)
  while (at !== -1) {
    out.push(at)
    at = hay.indexOf(needle, at + 1)
  }
  return out
}

/** Collapse runs of whitespace, so a reflowed paragraph still matches. */
function squash(s: string): string {
  return s.replace(/\s+/g, ' ').trim()
}

/**
 * Find an anchor in the current prose.
 *
 * Four passes, each weaker than the last, and the FIRST hit wins so a confident
 * match is never traded for a speculative one. Only the first pass reports
 * `exact`; everything else says `moved`, because the surroundings changed and a
 * reader deserves to know the note may have drifted in meaning.
 *
 * Returns null when the words are gone — the caller renders that as orphaned.
 */
export function resolveAnchor(paras: string[], a: Anchor): Placed | null {
  if (!a.quote) return null
  // Paired with their text so every pass has a definite string to search: an
  // index into a possibly-shorter array is exactly the kind of hole a note that
  // outlived its paragraph would fall through.
  const candidates = order(paras.length, a.para).flatMap((i) => {
    const text = paras[i]
    return text === undefined ? [] : [{ i, text }]
  })

  // 1 — prefix + quote + suffix, still adjacent. Certainly the same place.
  //
  // Only when there IS context. With both sides empty this search degenerates
  // into a bare-quote scan that returns the first hit and calls it exact — which
  // is precisely the coin flip pass 3 exists to refuse.
  const hasContext = a.prefix !== '' || a.suffix !== ''
  if (hasContext) {
    const full = a.prefix + a.quote + a.suffix
    for (const { i, text } of candidates) {
      const at = text.indexOf(full)
      if (at !== -1) {
        const start = at + a.prefix.length
        return { para: i, start, end: start + a.quote.length, how: 'exact' }
      }
    }
  }

  // 2 — one side of the context survived. Enough to disambiguate a repeated
  // sentence, not enough to promise the passage is unchanged.
  for (const { i, text } of candidates) {
    const withPrefix = a.prefix ? text.indexOf(a.prefix + a.quote) : -1
    if (withPrefix !== -1) {
      const start = withPrefix + a.prefix.length
      return { para: i, start, end: start + a.quote.length, how: 'moved' }
    }
    const withSuffix = a.suffix ? text.indexOf(a.quote + a.suffix) : -1
    if (withSuffix !== -1) {
      return { para: i, start: withSuffix, end: withSuffix + a.quote.length, how: 'moved' }
    }
  }

  // 3 — the bare quote, but only where it is UNAMBIGUOUS. Two occurrences in one
  // paragraph and no surviving context is a coin flip; a coin flip that renders
  // as a confident highlight is worse than admitting the note came loose.
  //
  // When the anchor never HAD context — a quote spanning a whole paragraph — a
  // unique match is the strongest evidence obtainable, so it counts as exact
  // rather than being reported as drift that did not happen.
  const bare: Placement = hasContext ? 'moved' : 'exact'
  for (const { i, text } of candidates) {
    const hits = occurrences(text, a.quote)
    const only = hits.length === 1 ? hits[0] : undefined
    if (only !== undefined) {
      return { para: i, start: only, end: only + a.quote.length, how: bare }
    }
  }

  // 4 — whitespace-insensitive, for prose that was merely reflowed. The offsets
  // are computed against the squashed text and then mapped back by counting
  // non-space characters, so the highlight still lands on real coordinates.
  const wantedSquashed = squash(a.quote)
  if (wantedSquashed) {
    for (const { i, text } of candidates) {
      const hit = mapSquashed(text, wantedSquashed)
      if (hit) return { para: i, start: hit.start, end: hit.end, how: 'moved' }
    }
  }

  return null
}

/**
 * Locate a whitespace-collapsed needle in the ORIGINAL text.
 *
 * Matching against `squash(text)` gives offsets into a string the reader never
 * sees, so they are walked back onto the original by stepping both strings in
 * lockstep. Only a unique match counts, for the same reason as pass 3.
 */
function mapSquashed(text: string, needle: string): { start: number; end: number } | null {
  // Original index of each character of the squashed string.
  const map: number[] = []
  let squashed = ''
  let pendingSpace = false
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i] ?? ''
    if (/\s/.test(ch)) {
      pendingSpace = squashed.length > 0
      continue
    }
    if (pendingSpace) {
      squashed += ' '
      map.push(i)
      pendingSpace = false
    }
    squashed += ch
    map.push(i)
  }
  const hits = occurrences(squashed, needle)
  const only = hits.length === 1 ? hits[0] : undefined
  if (only === undefined) return null
  const start = map[only]
  const lastCh = map[only + needle.length - 1]
  if (start === undefined || lastCh === undefined) return null
  return { start, end: lastCh + 1 }
}

// ---- the DOM half -------------------------------------------------------
//
// The rendered paragraph is not a single text node: citation marks are real
// elements, and they display a bare number while the prose they came from says
// `[3]`. So a browser offset cannot be used as a plain-text offset directly —
// walking the nodes and substituting each mark's SOURCE form is what keeps
// selection coordinates in the same space as `paraText`, which is what the
// anchors and the diff are written against.

/** Marks carry their source form here so the walker can substitute it. */
export const CITE_ATTR = 'data-cite-source'

/**
 * Plain-text offset of (node, offset) within `root`, counting a citation mark as
 * its source form rather than its rendered label. Returns the full text length
 * if the position is not inside `root`.
 */
export function plainOffsetIn(root: Node, node: Node, offset: number): number {
  let total = 0
  let found: number | null = null

  const walk = (n: Node): void => {
    if (found !== null) return
    if (n === node && n.nodeType !== Node.TEXT_NODE) {
      // A selection boundary placed on an element resolves to the start of its
      // `offset`-th child, which is the total accumulated up to that point.
      found = total + lengthOfChildren(n, offset)
      return
    }
    if (n.nodeType === Node.TEXT_NODE) {
      if (n === node) {
        found = total + Math.min(offset, (n.textContent ?? '').length)
        return
      }
      total += (n.textContent ?? '').length
      return
    }
    const el = n as Element
    const source = el.getAttribute?.(CITE_ATTR)
    if (source !== null && source !== undefined) {
      total += source.length
      return
    }
    for (const child of Array.from(n.childNodes)) {
      walk(child)
      if (found !== null) return
    }
  }

  walk(root)
  return found ?? total
}

/** Plain length of an element's first `count` children, marks substituted. */
function lengthOfChildren(el: Node, count: number): number {
  let total = 0
  const kids = Array.from(el.childNodes).slice(0, count)
  for (const k of kids) total += plainLengthOf(k)
  return total
}

/** Plain length of a node, counting a citation mark as its source form. */
export function plainLengthOf(n: Node): number {
  if (n.nodeType === Node.TEXT_NODE) return (n.textContent ?? '').length
  const el = n as Element
  const source = el.getAttribute?.(CITE_ATTR)
  if (source !== null && source !== undefined) return source.length
  let total = 0
  for (const child of Array.from(n.childNodes)) total += plainLengthOf(child)
  return total
}
