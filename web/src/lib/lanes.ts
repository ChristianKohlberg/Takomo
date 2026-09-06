import { api } from './api'

export interface LaneTicket { id: string; title: string; state: string; body?: string | null; links?: Record<string, string>; metadata?: unknown }
export interface Lane {
  id: string; project: string; title: string; purpose: string; context: string
  conversation_ref: string | null; archived: boolean; tickets: LaneTicket[]; handoff_count: number
}
export type HandoffKind = 'preparation' | 'implementation' | 'review'
export type Provider = 'codex' | 'claude'
export interface Handoff {
  id: string; lane: string; kind: HandoffKind; provider: Provider; instructions: string; ticket_ids: string[]
  target_revision: string | null; parent_handoff: string | null
  status: 'draft' | 'queued' | 'running' | 'completed' | 'failed' | 'cancelled'
  result: string | null; revision: string | null; conversation_ref: string | null; context_applied?: boolean | null
  snapshot: { lane: { title: string; purpose: string; context: string }; tickets: LaneTicket[] }
}
export interface HandoffDraft { kind: HandoffKind; provider: Provider; instructions: string; ticket_ids: string[]; target_revision?: string; parent_handoff?: string }
interface Page<T> { items: T[]; total: number; limit: number }
const json = (body: unknown, method = 'POST') => ({ method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
export const listLanes = (token: string, project: string, signal?: AbortSignal, offset = 0) => api<Page<Lane>>(token, `/projects/${encodeURIComponent(project)}/lanes?limit=200&offset=${offset}`, { signal })
export const createLane = (token: string, project: string, body: { title: string; purpose: string }) => api<Lane>(token, `/projects/${encodeURIComponent(project)}/lanes`, json(body))
export const getLane = (token: string, id: string, signal?: AbortSignal) => api<Lane>(token, `/lanes/${encodeURIComponent(id)}`, { signal })
export const updateLane = (token: string, id: string, body: Partial<Pick<Lane, 'title' | 'purpose' | 'context'>>) => api<Lane>(token, `/lanes/${encodeURIComponent(id)}`, json(body, 'PATCH'))
export const setLaneTicket = (token: string, id: string, ticket: string, add: boolean) => api(token, `/lanes/${encodeURIComponent(id)}/tickets/${encodeURIComponent(ticket)}`, { method: add ? 'PUT' : 'DELETE' })
export const listHandoffs = (token: string, id: string, signal?: AbortSignal, offset = 0) => api<Page<Handoff>>(token, `/lanes/${encodeURIComponent(id)}/handoffs?limit=200&offset=${offset}`, { signal })
export const createHandoff = (token: string, id: string, draft: HandoffDraft) => api<Handoff>(token, `/lanes/${encodeURIComponent(id)}/handoffs`, json(draft))
export const dispatchHandoff = (token: string, id: string) => api<Handoff>(token, `/handoffs/${encodeURIComponent(id)}/dispatch`, json({}))
export const cancelHandoff = (token: string, id: string) => api<Handoff>(token, `/handoffs/${encodeURIComponent(id)}/cancel`, json({}))
