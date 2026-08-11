// The /v1/initiatives surface, typed.
//
// Initiatives are the one thing in Takomo that is NOT work: an idea being
// nurtured, fed by appending entries over time, each recording where it came
// from. No workflow, no claim, no lease — `status` is a label, not a state
// machine. That is why this module has no transition helpers and never will.
//
// Entries are append-only on every surface, so there is deliberately no update
// or delete here either.
import { api } from './api'

export type InitiativeStatus = 'open' | 'parked' | 'distilled'

export const STATUSES: readonly InitiativeStatus[] = ['open', 'parked', 'distilled'] as const

/**
 * Conventional entry kinds, offered as suggestions rather than a fixed set: the
 * vocabulary is open by design (a new kind is just a new slug), so the UI
 * suggests without constraining.
 */
export const KINDS: readonly string[] = [
  'note',
  'research',
  'feedback',
  'transcript',
  'document',
  'decision',
] as const

export interface Rollup {
  entries?: number
  attachments?: number
  chars?: number
  bytes?: number
  megabytes?: number
  last_entry_at?: string | null
}

export interface Initiative {
  id: string
  project: string
  title: string
  summary?: string | null
  status: InitiativeStatus
  labels?: string[]
  tags?: string[]
  updated_at?: string
  rollup?: Rollup
}

export interface Entry {
  id: string
  initiative: string
  kind: string
  source: string
  title?: string | null
  text?: string | null
  source_uri?: string | null
  origin_at?: string | null
  created_at: string
  author: string
  has_content?: boolean
  filename?: string | null
  content_bytes?: number | null
  /**
   * Free-form JSON the appender attached. The server has always stored and
   * returned it (`meta` on the entry); the document view is the first reader to
   * mean anything by it — `pane`, `cites`, `para`. See lib/initiative-doc.ts.
   */
  meta?: unknown
}

export interface Page<T> {
  items: T[]
  next_cursor?: string | null
  rollup?: Rollup
}

export interface Project {
  id: string
  name?: string
  workflow?: string
}

export interface Whoami {
  actor: string
  scopes: string[]
  /**
   * This credential's own id — what `takomo token revoke` takes, and what tells
   * two tokens apart when they necessarily share an actor (one per machine,
   * minted by the same script). /settings uses it to mark the viewer's own row
   * and to withhold its Revoke button.
   */
  token_id?: string
  /**
   * `'*'` for an unrestricted token, else the project allowlist.
   *
   * The literal, not `null` — `/v1/whoami` serializes the unrestricted case as
   * the STRING `"*"`, the same way a token row does. This said `string[] | null`
   * until /settings became the first page to read it, where `projects.join()`
   * threw at runtime on a perfectly ordinary admin token: `"*"` has a `.length`
   * of 1, so a truthy-plus-length guard sails straight past it too.
   */
  projects?: '*' | string[]
}

const json = { 'Content-Type': 'application/json' }

export function listProjects(token: string): Promise<Project[]> {
  return api<Project[]>(token, '/projects')
}

export function whoami(token: string): Promise<Whoami> {
  return api<Whoami>(token, '/whoami')
}

export interface ListFilter {
  project?: string
  status?: string
  q?: string
  limit?: number
}

export function listInitiatives(token: string, f: ListFilter): Promise<Page<Initiative>> {
  const qs = new URLSearchParams({ limit: String(f.limit ?? 100) })
  if (f.project) qs.set('project', f.project)
  if (f.status) qs.set('status', f.status)
  if (f.q?.trim()) qs.set('q', f.q.trim())
  return api<Page<Initiative>>(token, `/initiatives?${qs}`)
}

export const ENTRY_PAGE = 50

export function listEntries(
  token: string,
  id: string,
  cursor?: string | null,
): Promise<Page<Entry>> {
  const qs = new URLSearchParams({ limit: String(ENTRY_PAGE) })
  if (cursor) qs.set('cursor', cursor)
  return api<Page<Entry>>(token, `/initiatives/${encodeURIComponent(id)}/entries?${qs}`)
}

export interface CreateFields {
  project: string
  title: string
  summary?: string
  labels?: string[]
  tags?: string[]
}

export function createInitiative(token: string, fields: CreateFields): Promise<Initiative> {
  return api<Initiative>(token, '/initiatives', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export type PatchFields = Partial<Pick<Initiative, 'title' | 'summary' | 'status'>>

export function patchInitiative(
  token: string,
  id: string,
  body: PatchFields,
): Promise<Initiative> {
  return api<Initiative>(token, `/initiatives/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(body),
  })
}

export interface AppendFields {
  kind: string
  source: string
  text?: string
  title?: string
  source_uri?: string
  origin_at?: string
  content_base64?: string
  filename?: string
  mime?: string
}

/**
 * Append an entry.
 *
 * Note the shape: this is the ONE route in this module that wraps its result —
 * it answers `{"entry": {…}}` where create and patch answer the record flat.
 * Verified against a running server, not inferred; the wrapper is unwrapped here
 * so callers see an `Entry` like everywhere else.
 */
export async function appendEntry(
  token: string,
  id: string,
  body: AppendFields,
): Promise<Entry> {
  const res = await api<{ entry: Entry }>(
    token,
    `/initiatives/${encodeURIComponent(id)}/entries`,
    { method: 'POST', headers: json, body: JSON.stringify(body) },
  )
  return res.entry
}

/**
 * Download an entry's attachment.
 *
 * The content route needs the bearer token, so a plain `<a href>` cannot fetch
 * it — the browser would send an unauthenticated request and get a 401. Fetch
 * with the header, then hand the blob to a throwaway object URL.
 */
export async function downloadAttachment(token: string, entry: Entry): Promise<void> {
  const path = `/v1/initiatives/${encodeURIComponent(entry.initiative)}/entries/${encodeURIComponent(entry.id)}/content`
  const r = await fetch(path, { headers: { Authorization: 'Bearer ' + token } })
  if (!r.ok) throw new Error('HTTP ' + r.status)
  const blob = await r.blob()
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = entry.filename || entry.id
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  setTimeout(() => URL.revokeObjectURL(url), 4000)
}

/**
 * Read a picked file into the base64 payload `content_base64` wants.
 *
 * `readAsDataURL` gives "data:<mime>;base64,<payload>" — the payload after the
 * comma is exactly what the API expects; JSON cannot carry bytes, so this is the
 * transport. The store's byte caps then bound the real payload rather than its
 * 4/3-inflated encoding.
 */
export function readFileAsBase64(
  file: File,
): Promise<{ name: string; mime: string; b64: string; size: number }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const res = String(reader.result || '')
      const comma = res.indexOf(',')
      resolve({
        name: file.name,
        mime: file.type || '',
        b64: comma >= 0 ? res.slice(comma + 1) : '',
        size: file.size,
      })
    }
    reader.onerror = () => reject(new Error('Could not read the file'))
    reader.readAsDataURL(file)
  })
}
