// Ranking and truncation for every search-as-you-type control.
//
// This was inline in Typeahead until a second control needed it. It is the part
// worth sharing: not the markup, which differs between a filter box and a
// navigation picker, but the two rules that are silently wrong when they rot —
// how results are ordered, and what the footer is allowed to claim about them.
//
// Count BEFORE truncating. The original code did `.filter().slice(0, 12)` and
// then reported `matches.length`, so the footer said "12 matches" whether there
// were 12 or 400 — the reader was told the list was complete when it was a
// fraction of it. `total` here is the count over the full match set; `shown` is
// what fits. A caller that conflates them is back to the same bug.

export interface RankableOption {
  id: string
  /** Secondary text searched alongside the id — a ticket title, a project name. */
  title?: string | null
}

export interface RankedResult<T> {
  /** Every match, ranked. */
  all: T[]
  /** The first `max` of them — what a list should render. */
  shown: T[]
  /** `all.length`. Named so a footer cannot accidentally report the truncated count. */
  total: number
}

/**
 * Rank deliberately cheaply and predictably: an exact id wins, then an id
 * prefix, then a title prefix, then anything else. No fuzzy matching — a filter
 * that reorders unpredictably is worse than one that does not.
 *
 * An empty query matches everything, in the order given.
 */
export function rankOptions<T extends RankableOption>(
  options: T[],
  query: string,
  max: number,
): RankedResult<T> {
  const q = query.trim().toLowerCase()
  if (!q) return { all: options, shown: options.slice(0, max), total: options.length }

  const hits = options.filter(
    (t) => t.id.toLowerCase().includes(q) || (t.title ?? '').toLowerCase().includes(q),
  )
  const rank = (t: T) => {
    const id = t.id.toLowerCase()
    const title = (t.title ?? '').toLowerCase()
    if (id === q) return 0
    if (id.startsWith(q)) return 1
    if (title.startsWith(q)) return 2
    if (id.includes(q)) return 3
    return 4
  }
  // A stable sort keeps server order within a rank, so equal-ranked results do
  // not shuffle as you type.
  const ranked = [...hits].sort((a, b) => rank(a) - rank(b))
  return { all: ranked, shown: ranked.slice(0, max), total: ranked.length }
}
