// The board's data: the project's workflow (which defines the columns), its
// tickets, and the event cursor that keeps them live.
//
// `/board` polls `GET /v1/events?since=<cursor>` rather than using the SSE
// stream, because the browser `EventSource` API cannot set an `Authorization`
// header and every fetch here carries a bearer token.
import { api } from './api'

export interface WorkflowState {
  id: string
  category?: string
  terminal?: boolean
}

export interface Workflow {
  name?: string
  initial?: string
  states: WorkflowState[]
  transitions?: unknown
  guards?: unknown
}

export interface Claim {
  holder?: string | null
  expires_at?: string | null
  fence_seq?: number
}

export interface Comment {
  author?: string
  body: string
  created_at?: string
}

export interface Promotion {
  target: string
  actor?: string
  created_at?: string
  url?: string | null
  ref?: string | null
  note?: string | null
}

export interface Ticket {
  id: string
  project: string
  type?: string
  parent?: string | null
  title: string
  body?: string | null
  state: string
  state_category?: string
  priority?: string
  labels?: string[]
  tags?: string[]
  claim?: Claim | null
  blocked_by?: string[]
  links?: Record<string, string>
  archived_at?: string | null
  created_at?: string
  updated_at?: string
  /** Set when a schedule created this ticket; it is the schedule's id. */
  schedule?: string | null
  occurrence?: string | null
  /** An occurrence's deadline. Passing it transitions nothing — see the card. */
  expires_at?: string | null
  /** Present only on the detail read — see `getTicket`. */
  comments?: Comment[]
  promotions?: Promotion[]
  deps?: { blocked_by?: (string | { id: string; title?: string; state?: string })[]; blocks?: (string | { id: string })[] }
}

export function getWorkflow(token: string, project: string): Promise<Workflow> {
  return api<Workflow>(token, `/projects/${encodeURIComponent(project)}/workflow`)
}

/** Every page of a project's tickets, with the same cursor guards as elsewhere. */
export async function listTickets(token: string, project: string): Promise<Ticket[]> {
  const out: Ticket[] = []
  let cursor: string | null = null
  for (let page = 0; page < 100 && out.length < 5000; page++) {
    const qs = new URLSearchParams({ project, limit: '200' })
    if (cursor != null) qs.set('cursor', cursor)
    const body: { items?: Ticket[]; next_cursor?: string | null } = await api(
      token,
      `/tickets?${qs}`,
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
 * The one ticket, with everything the drawer shows.
 *
 * `include=comments,deps,promotions` is asked for HERE and not on the list: the
 * board loads hundreds of tickets and pulling every comment thread to render
 * cards would be paying for the drawer on every row.
 */
export function getTicket(token: string, id: string): Promise<Ticket> {
  return api<Ticket>(token, `/tickets/${encodeURIComponent(id)}?include=comments,deps,promotions`)
}

export interface EventPage {
  items: { id?: number; kind?: string; ticket?: string }[]
  cursor?: number | string | null
}

/**
 * Events since a cursor. `since` is required — the server says so rather than
 * guessing — and the response's `cursor` is the next `since`.
 */
export function getEvents(token: string, since: number | string): Promise<EventPage> {
  return api<EventPage>(token, `/events?since=${encodeURIComponent(String(since))}&limit=200`)
}
