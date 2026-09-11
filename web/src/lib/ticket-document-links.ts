import { api } from './api'
export interface DocumentReference {
  id: string; ticket: string; mindmap: string; section_id: string; title: string; section_version: string
  relation: 'source' | 'related'; provenance: 'direct' | 'manual' | 'automatic'; state: 'suggested' | 'accepted' | 'removed'
  primary: boolean; reason?: string | null; quote?: string | null; created_by: string; created_at: string | number; captured_title?: string; reviewed_by?: string | null; reviewed_at?: string | number | null; missing: boolean; stale: boolean
  ticket_title?: string; ticket_state?: string
}
export interface TicketDocumentLinks {
  links: DocumentReference[]
  classification: { status: string; job_id?: string | null; error?: string | null; no_match_reason?: string | null; ambiguity?: string | boolean | null } | null
}
export interface ReferencePage { items: DocumentReference[]; total: number; limit: number; offset?: number }
export type ClassificationScheduling = 'off' | 'manual' | 'automatic'
export type ClassificationPolicy = 'suggest' | 'auto_apply_clear'
const base = (ticket: string) => `/tickets/${encodeURIComponent(ticket)}`
const json = { 'Content-Type': 'application/json' }
export const getTicketDocumentLinks = (token: string, ticket: string, signal?: AbortSignal) => api<TicketDocumentLinks>(token, `${base(ticket)}/document-links`, { signal })
export const addTicketDocumentLink = (token: string, ticket: string, section_id: string, primary: boolean, signal?: AbortSignal) => api(token, `${base(ticket)}/document-links`, { method: 'POST', headers: json, body: JSON.stringify({ section_id, primary }), signal })
export const changeTicketDocumentLink = (token: string, ticket: string, link: string, change: { state: 'accepted' | 'removed'; primary?: boolean }, signal?: AbortSignal) => api(token, `${base(ticket)}/document-links/${encodeURIComponent(link)}`, { method: 'PATCH', headers: json, body: JSON.stringify(change), signal })
export const classifyTicketDocument = (token: string, ticket: string, request_id: string, signal?: AbortSignal) => api(token, `${base(ticket)}/document-classification`, { method: 'POST', headers: json, body: JSON.stringify({ request_id }), signal })
export const getProjectDocumentLinks = (token: string, project: string, signal?: AbortSignal, section?: string, offset = 0) => api<ReferencePage>(token, `/projects/${encodeURIComponent(project)}/document-links?limit=500&offset=${offset}${section ? `&section_id=${encodeURIComponent(section)}` : ''}`, { signal })
export const getClassificationPolicy = (token: string, project: string, signal?: AbortSignal) => api<{ mode: ClassificationPolicy; scheduling?: ClassificationScheduling }>(token, `/projects/${encodeURIComponent(project)}/document-classification-config`, { signal })
export const saveClassificationPolicy = (token: string, project: string, mode: ClassificationPolicy, signal?: AbortSignal, scheduling?: ClassificationScheduling) => api<{ cancelled?: number }>(token, `/projects/${encodeURIComponent(project)}/document-classification-config`, { method: 'PUT', headers: json, body: JSON.stringify({ mode, ...(scheduling ? { scheduling } : {}) }), signal })
export async function documentSections(token: string, project: string, signal?: AbortSignal) {
  const maps = await api<{ items: { id: string }[] }>(token, `/mindmaps?project=${encodeURIComponent(project)}&limit=1`, { signal })
  if (!maps.items[0]) return []
  const map = await api<{ nodes: { id: string; text: string; parent: string | null; position: number }[] }>(token, `/mindmaps/${encodeURIComponent(maps.items[0].id)}`, { signal })
  return map.nodes.map(node => ({ id: node.id, title: node.text, parent: node.parent, position: node.position, order: String(node.position).padStart(8, '0') }))
}

export const classifyProjectDocuments = (token: string, project: string, request_id: string, signal?: AbortSignal) => api<{ scheduled: number }>(token, `/projects/${encodeURIComponent(project)}/document-classification`, { method: 'POST', headers: json, body: JSON.stringify({ request_id }), signal })
