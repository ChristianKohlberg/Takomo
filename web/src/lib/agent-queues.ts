import { api } from './api'

export const JOB_STATUSES = ['queued', 'running', 'completed', 'failed'] as const
export type AgentJobStatus = typeof JOB_STATUSES[number]
export interface AgentJob {
  id: string
  conversation_id: string
  project: string
  mindmap: string
  node: string
  section_title: string
  status: AgentJobStatus
  requested_by: string
  source_revision: string
  created_at: number
  finished_at: number | null
  lease_expires_at: number | null
  deadline: number | null
  service_id: string | null
  conversation_service_id: string | null
  attempt_id: string | null
  thread_id: string | null
  turn_id: string | null
  error: string | null
}
export interface AgentJobList {
  items: AgentJob[]
  counts: Record<AgentJobStatus, number>
  total: number
  limit: number
  note?: string
}
export interface AgentJobDetail {
  job: AgentJob & { prompt: string; snapshot: string; response: string | null }
  messages: { id: string; job_id: string; role: 'user' | 'assistant'; body: string; created_at: number }[]
}
export function listAgentJobs(token: string, project: string, status: AgentJobStatus | '', signal?: AbortSignal) {
  const query = new URLSearchParams({ limit: '100' })
  if (project) query.set('project', project)
  if (status) query.set('status', status)
  return api<AgentJobList>(token, `/agent-jobs?${query}`, { signal })
}
export function getAgentJob(token: string, id: string, signal?: AbortSignal) {
  return api<AgentJobDetail>(token, `/agent-jobs/${encodeURIComponent(id)}`, { signal })
}
