// Pure ticket-tree helpers, ported from board.html.
//
// They took `state.tickets` from the enclosing page scope before, which is the
// reason they could not be tested and could not be shared. They now take the
// lookup as an argument — the same discipline the library build enforces on
// components, applied to logic.
export interface TicketNode {
  id: string
  type?: string
  parent?: string | null
  /** Optional: the walks ignore it, but callers label groups with it. */
  title?: string
}

/** Ticket id -> ticket. Missing ids are tolerated: a parent can dangle. */
export type TicketIndex = Record<string, TicketNode | undefined>

// Both walks are guarded twice over — a visited-set against a malformed cycle,
// and a depth cap — because `parent` is server data and a cycle there must
// degrade the UI, never hang the tab.
const MAX_DEPTH = 30

/**
 * The id of a ticket's TOP-MOST epic ancestor, or '' when it has none.
 * An epic heads its own group. Nested epics therefore collapse into the
 * outermost one, which is what keeps the board's grouping stable when someone
 * files an epic under an epic.
 */
export function epicOf(t: TicketNode | undefined, index: TicketIndex): string {
  let cur = t
  let depth = 0
  const seen = new Set<string>()
  let lastEpic = ''
  while (cur && depth < MAX_DEPTH && !seen.has(cur.id)) {
    seen.add(cur.id)
    if (cur.type === 'epic') lastEpic = cur.id
    if (!cur.parent) break
    cur = index[cur.parent]
    depth++
  }
  return lastEpic
}

/**
 * True when `ancestorId` is `t` itself or any ancestor of it — i.e. `t` lies in
 * that ticket's subtree. Used by both the epic filter and the ticket filter, so
 * "filter by TK-7" keeps TK-7's subtasks visible instead of orphaning them.
 */
export function inSubtree(
  t: TicketNode | undefined,
  ancestorId: string,
  index: TicketIndex,
): boolean {
  let cur = t
  let depth = 0
  const seen = new Set<string>()
  while (cur && depth < MAX_DEPTH && !seen.has(cur.id)) {
    seen.add(cur.id)
    if (cur.id === ancestorId) return true
    if (!cur.parent) break
    cur = index[cur.parent]
    depth++
  }
  return false
}

/** Build the id -> ticket index both walks take. */
export function indexById(tickets: readonly TicketNode[]): TicketIndex {
  const out: TicketIndex = {}
  for (const t of tickets) out[t.id] = t
  return out
}
