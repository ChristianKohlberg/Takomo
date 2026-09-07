import { api } from './api'
import type { LaneTicket } from './lanes'

export interface OrganizerGroup {
  lane_id: string | null
  title: string
  purpose: string
  context: string
  readiness: 'ready' | 'needs_clarification'
  reason: string
  ticket_ids: string[]
}
export interface OrganizerJob {
  id: string
  status: 'queued' | 'running' | 'completed' | 'failed'
  error: string | null
  created_at: string
  accepted_at: string | null
  proposal: { groups: OrganizerGroup[]; unassigned: { ticket_id: string; reason: string }[] } | null
  snapshot: { tickets: LaneTicket[]; lanes: { id: string; title: string; purpose: string; context: string; tickets: string[] }[] }
}
export interface OrganizerView {
  conversation_id: string | null
  messages: { id: string; role: string; body: string; created_at: string }[]
  jobs: OrganizerJob[]
  total: number
  limit: number
}
const path = (project: string) => `/projects/${encodeURIComponent(project)}/lane-organizer`
const json = (body: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
export const getOrganizer = (token: string, project: string, signal?: AbortSignal) => api<OrganizerView>(token, path(project), { signal })
export const requestOrganization = (token: string, project: string, body: { request_id: string; message: string }) => api<OrganizerView>(token, `${path(project)}/messages`, json(body))
export const acceptOrganization = (token: string, project: string, id: string) => api<OrganizerView>(token, `${path(project)}/jobs/${encodeURIComponent(id)}/accept`, json({}))
