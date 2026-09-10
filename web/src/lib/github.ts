import { api } from './api'
export interface Installation { id: number; account: string; management_url?: string; suspended?: boolean }
export interface GithubStatus { configured: boolean; app_slug: string | null; connections: Installation[] }
export interface Repository { id: number; full_name: string; private: boolean }
export interface RepositorySelection { installation: number; repository: number; full_name: string; scope: { include: string[]; exclude: string[] } }
export interface ExtractionRun { id: string; mindmap: string; status: 'queued' | 'running' | 'completed' | 'failed'; error: string | null }
export const githubStatus = (token: string) => api<GithubStatus>(token, '/integrations/github')
export const githubInstallations = (token: string) => api<{ items: Installation[]; has_more: boolean }>(token, '/integrations/github/installations')
export const githubRepositories = (token: string, id: number, page = 1) => api<{ items: Repository[]; total: number }>(token, `/integrations/github/installations/${id}/repositories?page=${page}`)
export function githubWrite<T = unknown>(token: string, path: string, body: unknown, method = 'POST') {
  return api<T>(token, path, { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
}
export const connectGithub = (token: string, installation: number) => githubWrite(token, '/integrations/github/installations', { installation })
export const disconnectGithub = (token: string, id: number) => api(token, `/integrations/github/installations/${id}`, { method: 'DELETE' })
export const setRepository = (token: string, project: string, selection: RepositorySelection) => githubWrite(token, `/projects/${encodeURIComponent(project)}/repository`, selection, 'PUT')
export const getRepository = (token: string, project: string) => api<{ repository: RepositorySelection | null }>(token, `/projects/${encodeURIComponent(project)}/repository`)
export const extractionRuns = (token: string, project: string) => api<{ items: ExtractionRun[] }>(token, `/projects/${encodeURIComponent(project)}/codebase-imports`)
export const startExtraction = (token: string, project: string, mindmap: string, request_id: string) => githubWrite<{ id: string }>(token, `/projects/${encodeURIComponent(project)}/codebase-imports`, { mindmap, request_id })
