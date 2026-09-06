import { api } from './api'
import type { Ticket } from './board'
export interface BugJob { id: string; status: string; created_at: number; response?: string | null; prompt?: string; snapshot?: string; error?: string | null; source_revision?: string; repository_revision?: string | null; repository_ref?: {repository: string; revision: string}; evidence?: {inspected?: {path: string; start_line: number; end_line: number; revision: string}[]; runtime_reproduced?: boolean} | null; steering?: {id: number; message: string}[] }
export interface Bug { ticket: Ticket; triage: string; severity: string; duplicate_of: string | null; latest_job: BugJob | null; note?: string | null; updated_by?: string | null; updated_at?: number | null }
export interface BugPage { items: Bug[]; total: number; limit: number; offset: number }
export interface ResearchConfig { repository: string; revision: string; enabled: boolean }
export const json = (body: unknown) => ({ headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
export const bugPath = (id: string) => `/bugs/${encodeURIComponent(id)}`
export function listBugs(token: string, project: string, view: string, severity: string, search: string, offset: number, signal?: AbortSignal, filters?: {state: string; assignee: string; research_status: string}) {
  const query = new URLSearchParams({ project, view, limit: '50', offset: String(offset) })
  if (severity) query.set('severity', severity)
  if (search) query.set('search', search)
  if (filters) for (const [key, value] of Object.entries(filters)) if (value) query.set(key, value)
  return api<BugPage>(token, `/bugs?${query}`, { signal })
}
