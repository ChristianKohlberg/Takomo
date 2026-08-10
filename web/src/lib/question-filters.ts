// What the /inbox filter bar admits.
//
// Kept out of the page so it is testable without a DOM: the folder counts and
// the list both read from `filterQuestions`, so a question that is filtered out
// cannot be counted in a folder it is not listed in.
import type { Question } from './questions'

export interface QuestionQuery {
  /** Exact ticket id, from the ticket typeahead. */
  ticket?: string
  /** Free text. Whitespace-separated terms, ALL of which must match. */
  search?: string
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

export function filterQuestions(questions: Question[], query: QuestionQuery): Question[] {
  let out = questions
  if (query.ticket) out = out.filter((q) => q.ticket === query.ticket)
  if (query.search?.trim()) out = out.filter((q) => matchesSearch(q, query.search!))
  return out
}

/** How many filters are active — the badge on the phone's collapsed bar. */
export function activeFilterCount(query: QuestionQuery): number {
  return (query.ticket ? 1 : 0) + (query.search?.trim() ? 1 : 0)
}
