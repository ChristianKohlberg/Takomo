// The two credential-bearing modes of /board.
//
// Both are deliberately NOT the `tk_` API: a share token reaches only
// `/v1/shares/self*` and an answer grant only `/v1/answer/self`, enforced by
// their own middleware rather than by scope checks inside the normal routes. A
// token of one kind structurally cannot reach another's surface.
//
// Shapes verified against a running server.
import { api } from './api'
import type { Question } from './questions'

// ---------------------------------------------------------------------------
// Share (`tks_`) — read-only.

export interface ShareSelf {
  kind: string
  ref?: string | null
  project: string
  expires_at?: string | null
  workflow?: {
    name?: string
    initial?: string
    states?: { id: string; category?: string; terminal?: boolean }[]
  }
}

export interface ShareTicket {
  id: string
  title: string
  state: string
  priority?: string
  type?: string
  parent?: string | null
  body?: string | null
  labels?: string[]
  tags?: string[]
  claim?: unknown
  blocked_by?: string[]
  created_at?: string
  updated_at?: string
}

export function shareSelf(token: string): Promise<ShareSelf> {
  return api<ShareSelf>(token, '/shares/self')
}

/**
 * Every page of the shared subtree. Same cursor discipline as the inbox: a
 * repeated cursor or an empty page ends the loop, with hard caps, because a bad
 * cursor is otherwise an infinite loop in the reader's tab.
 */
export async function shareTickets(token: string): Promise<ShareTicket[]> {
  const out: ShareTicket[] = []
  let cursor: string | null = null
  for (let page = 0; page < 100 && out.length < 5000; page++) {
    const qs = new URLSearchParams({ limit: '200' })
    if (cursor != null) qs.set('cursor', cursor)
    const body: { items?: ShareTicket[]; next_cursor?: string | null } = await api(
      token,
      `/shares/self/tickets?${qs}`,
    )
    const got = body.items ?? []
    out.push(...got)
    const next = body.next_cursor ?? null
    if (next == null || next === cursor || got.length === 0) break
    cursor = next
  }
  return out
}

export function shareTicket(token: string, id: string): Promise<ShareTicket> {
  return api<ShareTicket>(token, `/shares/self/tickets/${encodeURIComponent(id)}`)
}

// ---------------------------------------------------------------------------
// Answer grant (`tka_`) — read and answer exactly one question, then spent.

export interface AnswerGrant {
  expires_at?: string | null
  question: Question
  ticket?: {
    id: string
    title?: string
    state?: string
    body?: string | null
  } | null
}

export function answerGrantSelf(token: string): Promise<AnswerGrant> {
  return api<AnswerGrant>(token, '/answer/self')
}

/**
 * Spend the grant.
 *
 * Single-use is atomic server-side: the link is marked used in the same
 * transaction that records the answer, so of any number of simultaneous
 * submissions exactly one lands and the rest get 410. A rejected answer rolls
 * the spend back, so a link is never burned by an attempt that did not apply —
 * which is why a 4xx here should leave the reader able to try again.
 */
export function submitGrantAnswer(token: string, answer: unknown): Promise<unknown> {
  return api(token, '/answer/self', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ answer }),
  })
}
