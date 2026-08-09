// The /v1/questions surface — ask-a-human.
//
// A `blocking` question parks its ticket and releases the agent's lease;
// answering resumes it through the workflow's human-gated edge, but only once
// EVERY open blocking question on that ticket is answered (a barrier). An
// `advisory` question records a routed decision and never touches ticket state.
// That distinction is why the snackbar says "resumed into <ticket>" for one and
// "recorded" for the other.
//
// Shapes verified against a running server.
import { api } from './api'

export type QuestionKind = 'confirm' | 'choose' | 'clarify' | 'approve'
export type QuestionMode = 'blocking' | 'advisory'
export type QuestionStatus = 'open' | 'answered' | 'withdrawn' | 'expired'
export type Folder = QuestionStatus

export const FOLDERS: readonly Folder[] = ['open', 'answered', 'withdrawn', 'expired'] as const

export interface Answer {
  value?: unknown
  custom?: boolean
  note?: string | null
}

export interface Question {
  id: string
  project: string
  ticket: string
  kind: QuestionKind
  mode: QuestionMode
  status: QuestionStatus
  title: string
  body?: string | null
  summary?: string | null
  /** Preset choices for `choose`. Empty for every other kind. */
  options: string[]
  /** Per-option rationale, positionally aligned with `options`. */
  option_notes: string[]
  multi: boolean
  recommended?: unknown
  recommended_multi: string[]
  recommended_note?: string | null
  answer?: Answer | null
  answered_by?: string | null
  answered_at?: string | null
  asked_by?: string | null
  /** `human` = waiting on a person; `agent` = bounced back for more research. */
  awaiting?: 'human' | 'agent' | null
  expertise: string[]
  urgency?: string | null
  confidence?: number | null
  on_timeout?: string | null
  expires_at?: string | null
  resolved_to?: string | null
  created_at: string
  updated_at?: string
  version?: number
}

export interface ThreadMessage {
  id?: string
  question?: string
  role: 'human' | 'agent'
  /** The message text. Named `body`, like everywhere else in the store. */
  body: string
  author?: string | null
  created_at: string
}

const json = { 'Content-Type': 'application/json' }

export interface QuestionFilter {
  project?: string
  /** Comma-separated statuses. Defaults to all four folders. */
  status?: string
  limit?: number
}

/**
 * The list defaults to OPEN ONLY — verified against a running server. The inbox
 * needs every folder to show a count, so it asks for all four by name; there is
 * no "all".
 */
const ALL_STATUSES = 'open,answered,withdrawn,expired'

/**
 * Every page of the list.
 *
 * The loop stops unless the server both signals more AND is actually advancing:
 * a repeated cursor or an empty page ends it, and hard caps on pages and items
 * backstop a server that misbehaves. Without those a bad cursor is an infinite
 * loop in the reader's tab.
 */
export async function listQuestions(token: string, f: QuestionFilter = {}): Promise<Question[]> {
  const out: Question[] = []
  let cursor: string | null = null
  for (let page = 0; page < 100 && out.length < 5000; page++) {
    const qs = new URLSearchParams({
      limit: String(f.limit ?? 200),
      status: f.status ?? ALL_STATUSES,
    })
    if (f.project) qs.set('project', f.project)
    if (cursor != null) qs.set('cursor', cursor)

    const body: { items?: Question[]; next_cursor?: string | null } = await api(
      token,
      `/questions?${qs}`,
    )
    const got = body.items ?? []
    out.push(...got)
    const next = body.next_cursor ?? null
    if (next == null || next === cursor || got.length === 0) break
    cursor = next
  }
  return out
}

/**
 * The question with its follow-up thread. The detail read carries `thread`;
 * the list read does not, which is why the reading pane fetches it.
 */
export function getQuestion(token: string, id: string): Promise<Question & { thread?: ThreadMessage[] }> {
  return api<Question & { thread?: ThreadMessage[] }>(token, `/questions/${encodeURIComponent(id)}`)
}

export function getThread(token: string, id: string): Promise<ThreadMessage[]> {
  return getQuestion(token, id).then((q) => q.thread ?? [])
}

/**
 * The answer payload.
 *
 * Two wire facts, both learned from the API's own refusals rather than guessed:
 *
 *   - the field is `answer`, NOT `value` ("Unknown field(s): value. Accepted:
 *     answer, resume_to, on_behalf_of.");
 *   - a note is NOT a sibling of `answer` — it nests inside it, so the body is
 *     `{answer: {value, note}}`. A top-level `note` is refused the same way.
 *
 * `custom` is carried locally for the optimistic render and is not sent.
 */
export interface AnswerPayload {
  value: unknown
  custom?: boolean
  note?: string
}

export function answerQuestion(
  token: string,
  id: string,
  payload: AnswerPayload,
): Promise<Question> {
  return api<Question>(token, `/questions/${encodeURIComponent(id)}/answer`, {
    method: 'POST',
    headers: json,
    // A bare scalar when there is no note; `{value, note}` when there is.
    body: JSON.stringify({
      answer: payload.note ? { value: payload.value, note: payload.note } : payload.value,
    }),
  })
}

/** Bounce it back to the asking agent for more research; the ticket stays parked. */
export function sendFollowup(token: string, id: string, text: string): Promise<unknown> {
  return api(token, `/questions/${encodeURIComponent(id)}/followup`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify({ text }),
  })
}

export function withdrawQuestion(token: string, id: string, reason?: string): Promise<unknown> {
  return api(token, `/questions/${encodeURIComponent(id)}/withdraw`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(reason ? { reason } : {}),
  })
}

export function reopenQuestion(token: string, id: string): Promise<unknown> {
  return api(token, `/questions/${encodeURIComponent(id)}/reopen`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

export interface AnswerLink {
  url?: string
  token?: string
  expires_at?: string
  ttl_seconds?: number
  ttl_source?: string
  warning?: string
}

/**
 * A per-question answer grant: a scoped, expiring, single-use URL that lets an
 * outside expert answer exactly this one question and nothing else.
 */
export function mintAnswerLink(token: string, id: string): Promise<AnswerLink> {
  return api<AnswerLink>(token, `/questions/${encodeURIComponent(id)}/answer-link`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

export interface TicketRef {
  id: string
  title?: string | null
  tags?: string[]
}

/**
 * The tickets the ticket filter matches on.
 *
 * `fields=id,title,tags` is deliberate and asserted by the Rust suite: without
 * `title` the filter has nothing but ids to match on, and asking for the whole
 * ticket would pull every body across the wire to populate a dropdown.
 */
export async function listTicketRefs(token: string, project?: string): Promise<TicketRef[]> {
  // Paginated, like `listQuestions` above and `listTickets` in lib/board.ts.
  //
  // This used to be a single `limit=500` request with no loop, so on a project
  // with more than 500 tickets the inbox's filter simply could not find the
  // later ones — and the failure is indistinguishable from "that ticket does
  // not exist", which is the worst way for a filter to fail.
  const out: TicketRef[] = []
  let cursor: string | null = null
  for (let page = 0; page < 100 && out.length < 5000; page++) {
    // Written as a literal rather than assembled: `fields=id,title,tags` is an
    // asserted contract, and a contract you cannot grep for is one that breaks
    // silently.
    const qs = new URLSearchParams({ limit: '500' })
    if (project) qs.set('project', project)
    if (cursor != null) qs.set('cursor', cursor)
    const body: { items?: TicketRef[]; next_cursor?: string | null } | TicketRef[] = await api(
      token,
      `/tickets?fields=id,title,tags&${qs}`,
    )
    // The endpoint answers either shape depending on projection.
    if (Array.isArray(body)) return body
    const got = body.items ?? []
    out.push(...got)
    const next = body.next_cursor ?? null
    if (next == null || next === cursor || got.length === 0) break
    cursor = next
  }
  return out
}

export interface AskFields {
  ticket: string
  kind: QuestionKind
  mode: QuestionMode
  title: string
  body?: string
  options?: string[]
  expertise?: string[]
}

/**
 * Ask a human, from the board.
 *
 * A `blocking` question parks the ticket and releases the agent's lease; an
 * `advisory` one records a routed decision and changes no ticket state. The
 * caller picks — this only carries it.
 */
export function askQuestion(token: string, fields: AskFields): Promise<Question> {
  return api<Question>(token, '/questions', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}
