// What the /inbox filter bar admits, and how the surviving questions are grouped.
//
// Kept out of the page so it is testable without a DOM: the folder counts and
// the list both read from `filterQuestions`, so a question that is filtered out
// cannot be counted in a folder it is not listed in.
import type { Question } from './questions'
import { epicOf, inSubtree, type TicketIndex } from './tickets'

export const URGENCIES = ['critical', 'high', 'normal', 'low'] as const
export type Urgency = (typeof URGENCIES)[number]

/** "Expiring soon" — a question inside this window of its `expires_at`. */
export const SOON_MS = 24 * 60 * 60 * 1000

export interface QuestionQuery {
  /** A ticket id. Subtree-aware when an index is supplied — see below. */
  ticket?: string
  /** Free text. Whitespace-separated terms, ALL of which must match. */
  search?: string
  /** Urgency levels to keep. Empty = every level. */
  urgency?: string[]
  /** `blocking` parks a ticket, `advisory` records a decision. '' = both. */
  mode?: 'blocking' | 'advisory' | ''
  /**
   * Everything waiting on the reader, in both senses this board has: addressed
   * to them by name, OR covered by one of their `expert:<tag>` scopes.
   *
   * A union, not an intersection. Requiring both would hide a question aimed
   * straight at somebody because it carried no routing tag — the very question
   * they most owe an answer on.
   */
  mine?: boolean
  /**
   * One person's queue, by user handle — including somebody else's, which is the
   * point of a shared board. `'none'` is the triage read: what nobody has been
   * asked yet.
   */
  assignee?: string
  /** Drop questions bounced back to the agent — they are not waiting on you. */
  hideAwaitingAgent?: boolean
  /** Only questions whose `expires_at` falls inside SOON_MS. */
  expiringSoon?: boolean
  /** An exact `asked_by` actor. */
  askedBy?: string
}

export interface FilterContext {
  /** Ticket id -> ticket, for the subtree walk. Absent = exact ticket match. */
  index?: TicketIndex
  /** The reader's expertise tags, from their `expert:<tag>` scopes. */
  expertise?: string[]
  /** The reader's own user handle, from `whoami`. Absent = a machine token. */
  handle?: string
  /** Injected so the "expiring soon" window is testable. */
  now?: number
}

/**
 * The text a search term is matched against.
 *
 * Deliberately wider than the row renders: the reader searching for a phrase
 * they remember from the question body would otherwise be told there is no
 * match, which is indistinguishable from "that question does not exist".
 */
function haystack(q: Question): string {
  return [
    q.title,
    q.summary ?? '',
    q.body ?? '',
    q.ticket,
    q.asked_by ?? '',
    q.kind,
    q.urgency ?? '',
    // Both spellings of the person: somebody searching for "ada" and somebody
    // searching for "Ada Lovelace" are looking for the same queue.
    q.assignee?.handle ?? '',
    q.assignee?.name ?? '',
    ...(q.expertise ?? []),
    ...(q.options ?? []),
  ]
    .join('\n')
    .toLowerCase()
}

export function matchesSearch(q: Question, search: string): boolean {
  const terms = search.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const hay = haystack(q)
  return terms.every((t) => hay.includes(t))
}

/** Milliseconds until a question auto-resolves, or null when it never does. */
export function msUntilExpiry(q: Question, now: number): number | null {
  if (!q.expires_at) return null
  const at = Date.parse(q.expires_at)
  return Number.isNaN(at) ? null : at - now
}

export function isExpiringSoon(q: Question, now: number, within = SOON_MS): boolean {
  const left = msUntilExpiry(q, now)
  return left !== null && left <= within
}

export function filterQuestions(
  questions: Question[],
  query: QuestionQuery,
  ctx: FilterContext = {},
): Question[] {
  const now = ctx.now ?? Date.now()
  let out = questions

  if (query.ticket) {
    // Subtree-aware, matching /board: filtering to an epic keeps the questions
    // asked against its children. Questions hang off the LEAVES, so an exact
    // match on an epic shows an empty inbox — with no way to tell that from
    // "nobody has asked anything about this epic".
    const index = ctx.index
    out = index
      ? out.filter((q) => inSubtree(index[q.ticket] ?? { id: q.ticket }, query.ticket!, index))
      : out.filter((q) => q.ticket === query.ticket)
  }
  if (query.search?.trim()) out = out.filter((q) => matchesSearch(q, query.search!))
  if (query.urgency?.length) {
    const want = new Set(query.urgency)
    // An absent urgency IS normal, server-side and in the row's left rule.
    out = out.filter((q) => want.has(q.urgency ?? 'normal'))
  }
  if (query.mode) out = out.filter((q) => q.mode === query.mode)
  if (query.mine) {
    const mine = new Set(ctx.expertise ?? [])
    // Either sense of waiting-on-you. Being nobody and covering nothing means an
    // empty list is the honest answer, and the bar disables the toggle rather
    // than showing it.
    out = out.filter(
      (q) =>
        (ctx.handle !== undefined && q.assignee?.handle === ctx.handle) ||
        (q.expertise ?? []).some((e) => mine.has(e)),
    )
  }
  if (query.assignee) {
    out =
      query.assignee === 'none'
        ? out.filter((q) => !q.assignee)
        : out.filter((q) => q.assignee?.handle === query.assignee)
  }
  if (query.hideAwaitingAgent) out = out.filter((q) => q.awaiting !== 'agent')
  if (query.expiringSoon) out = out.filter((q) => isExpiringSoon(q, now))
  if (query.askedBy) out = out.filter((q) => q.asked_by === query.askedBy)

  return out
}

/** How many filters are active — the badge on the phone's collapsed bar. */
export function activeFilterCount(query: QuestionQuery): number {
  return (
    (query.ticket ? 1 : 0) +
    (query.search?.trim() ? 1 : 0) +
    (query.urgency?.length ? 1 : 0) +
    (query.mode ? 1 : 0) +
    (query.mine ? 1 : 0) +
    (query.assignee ? 1 : 0) +
    (query.hideAwaitingAgent ? 1 : 0) +
    (query.expiringSoon ? 1 : 0) +
    (query.askedBy ? 1 : 0)
  )
}

export const EMPTY_QUERY: QuestionQuery = {}

/**
 * Sort a folder's questions.
 *
 * Open keeps the server's order (urgency, then oldest first) — that IS the
 * triage order. Every closed folder is sorted most-recent-first instead: the
 * server orders those by urgency and creation too, so "Answered" opened on the
 * oldest decision anyone ever made, which is never what a reader wants there.
 */
export function sortForFolder(questions: Question[], folder: string): Question[] {
  if (folder === 'open') return questions
  const stamp = (q: Question) => Date.parse(q.answered_at ?? q.updated_at ?? q.created_at) || 0
  return [...questions].sort((a, b) => stamp(b) - stamp(a))
}

export interface EpicGroup {
  /** The epic's ticket id, or '' for the questions under no epic. */
  epic: string
  /** The epic's title when the index knows it, else its id. */
  title: string
  questions: Question[]
}

/**
 * Group questions by the epic their ticket belongs to.
 *
 * Order follows the questions, not the epics: the first group is the one
 * holding the most urgent question, because the list is already sorted by the
 * order a reader should work through it. Regrouping by anything else would
 * silently reorder triage. The "no epic" bucket sorts last — it is a remainder,
 * not a topic.
 */
export function groupByEpic(questions: Question[], index: TicketIndex): EpicGroup[] {
  const groups = new Map<string, EpicGroup>()
  for (const q of questions) {
    const epic = epicOf(index[q.ticket], index)
    let g = groups.get(epic)
    if (!g) {
      g = { epic, title: (epic && index[epic]?.title) || epic, questions: [] }
      groups.set(epic, g)
    }
    g.questions.push(q)
  }
  const out = [...groups.values()]
  return out.sort((a, b) => (a.epic === '' ? 1 : 0) - (b.epic === '' ? 1 : 0))
}
