// Environments: the registry of places software can be run — a URL, how to
// bring it up, and whether writing there is safe. Takomo executes nothing.
//
// List reads return the server's paged envelope (`items`/`total`/`limit`, plus
// a prose `note` when the page left rows out).
import { api } from './api'

export const ENVIRONMENT_KINDS = [
  'local',
  'ephemeral',
  'shared',
  'staging',
  'production',
  'other',
] as const
export const ENVIRONMENT_DATA_STATES = ['seeded', 'empty', 'production_like', 'unknown'] as const

export type EnvironmentKind = (typeof ENVIRONMENT_KINDS)[number]

export interface Environment {
  id: string
  project: string
  slug: string
  name: string
  kind: EnvironmentKind
  base_url: string | null
  bring_up: string
  teardown: string
  data_state: string
  /** ADVISORY. Takomo executes nothing and cannot enforce it. */
  writable: boolean
  /** A POINTER to where a credential lives. Never a credential. */
  credentials_hint: string | null
  notes: string
  archived_at: string | null
  version: number
}

export interface Paged<T> {
  items: T[]
  total: number
  limit: number
  note?: string
}

const json = { 'Content-Type': 'application/json' }
const enc = encodeURIComponent

export function listEnvironments(
  token: string,
  project: string,
  includeArchived = false,
): Promise<Paged<Environment>> {
  const tail = includeArchived ? '?archived=include' : ''
  return api<Paged<Environment>>(token, `/projects/${enc(project)}/environments${tail}`)
}

export interface EnvironmentFields {
  slug: string
  name?: string
  kind?: EnvironmentKind
  base_url?: string
  bring_up?: string
  teardown?: string
  data_state?: string
  writable?: boolean
  credentials_hint?: string
  notes?: string
}

export function createEnvironment(
  token: string,
  project: string,
  f: EnvironmentFields,
): Promise<Environment> {
  return api<Environment>(token, `/projects/${enc(project)}/environments`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(f),
  })
}

/** No `slug`: agents and tool calls address an environment by it, so it is immutable. */
export function patchEnvironment(
  token: string,
  id: string,
  fields: Record<string, unknown>,
): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function archiveEnvironment(token: string, id: string): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}`, { method: 'DELETE' })
}

export function unarchiveEnvironment(token: string, id: string): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}/unarchive`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}
