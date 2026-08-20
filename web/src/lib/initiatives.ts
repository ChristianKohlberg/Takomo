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
import type { UserRef } from './users'

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
  /**
   * The attention pair: what this document is waiting on a person for. Every
   * other field here is volume, which never says where to look next.
   *
   * The server derives them with the same rules {@link buildDoc} applies — a note
   * stops counting once resolved or superseded, an amendment once decided — so a
   * row's badge and the document you open from it agree. They are the only part
   * of the document reachable without fetching that document's entries, which is
   * what lets a collection be read at all.
   */
  open_notes?: number
  pending_amendments?: number
}

/** What a document is waiting on a person for, from its rollup alone. */
export function waiting(rollup: Rollup | undefined): { notes: number; amendments: number } {
  return { notes: rollup?.open_notes ?? 0, amendments: rollup?.pending_amendments ?? 0 }
}

/**
 * Whether a document wants something from a person.
 *
 * A server too old to send the pair reports neither, so every document reads as
 * quiet — the honest degradation. Inventing attention from a missing field would
 * send readers into documents with nothing in them.
 */
export function isWaiting(i: { rollup?: Rollup }): boolean {
  const w = waiting(i.rollup)
  return w.notes > 0 || w.amendments > 0
}

/**
 * How many documents in a collection want something from a person.
 *
 * DOCUMENTS, not notes: the badge answers "how many places do I need to go",
 * and one document with nine open notes is still one place. Summing the notes
 * instead would make a single busy document look like a backlog.
 */
export function countWaiting(items: readonly { rollup?: Rollup }[]): number {
  return items.filter(isWaiting).length
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
  /**
   * Free-form JSON on the initiative itself, stored and returned by the server
   * since it existed. The explorer is its first reader: `metadata.path` is the
   * folder a document is filed in, which is why nesting needed no schema change.
   * See lib/initiative-tree.ts.
   */
  metadata?: unknown
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
  /**
   * True while the project is archived: a gate, not a status. Every write under
   * it is refused and its tickets are out of the ready queue; reads are
   * unaffected and nothing was deleted, so it is reversible.
   */
  archived?: boolean
  archived_at?: string | null
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
  /**
   * The person in the directory this credential belongs to, or null for a machine
   * token. A different kind of fact from `actor`, which is a free-form string the
   * credential carries: this one the server can vouch for, which is why it is what
   * "for me" reads on /inbox.
   */
  user?: UserRef | null
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
  /** `{ path }` files the new document into a folder as it is created. */
  metadata?: Record<string, unknown>
}

export function createInitiative(token: string, fields: CreateFields): Promise<Initiative> {
  return api<Initiative>(token, '/initiatives', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export type PatchFields = Partial<Pick<Initiative, 'title' | 'summary' | 'status'>> & {
  /**
   * Merged into the initiative's metadata rather than replacing it — a key set
   * to null is removed. This is how a document is moved between folders, and
   * merging is what keeps a move from discarding metadata some other writer put
   * there.
   */
  metadata_merge?: Record<string, unknown>
}

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

/** What a delete removed, and what it deliberately left standing. */
export interface DeletedInitiative {
  id: string
  project: string
  entries: number
  bytes: number
  /** Checks detached rather than deleted; only ever non-zero with `force`. */
  detached_checks: number
  /**
   * Tickets whose `initiative:<id>` tag now names nothing. Untouched — the
   * roadmap keeps them visible under `uninitiated` — but worth telling the
   * reader about, because a lane is about to disappear from the map.
   */
  tagged_tickets: number
}

/**
 * Delete an initiative and its entries.
 *
 * `force` detaches the verification checks filed under it instead of refusing.
 * Without it a single check answers `409 conflict.initiative_has_checks`, which
 * the page turns into a second, explicit confirmation rather than retrying
 * behind the reader's back.
 */
export function deleteInitiative(
  token: string,
  id: string,
  force = false,
): Promise<DeletedInitiative> {
  const q = force ? '?force=true' : ''
  return api<DeletedInitiative>(token, `/initiatives/${encodeURIComponent(id)}${q}`, {
    method: 'DELETE',
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
  /**
   * Free-form JSON stored on the entry. The document view is what reads it —
   * `pane`, `cites`, `para`, `proposed`, `supersedes`, `accepts`/`rejects`. See
   * lib/initiative-doc.ts.
   */
  meta?: unknown
}

/**
 * File a ticket. The document view uses this to dispatch a margin note: the
 * fleet then pulls it off the ready queue it already pulls from, so acting on a
 * note needs no new execution machinery on the server.
 */
export async function createTicket(
  token: string,
  body: { project: string; title: string; body?: string; tags?: string[]; ty?: string },
): Promise<string> {
  // The create route answers the ticket FLAT — `{ id, project, title, … }` —
  // not wrapped the way the entry-append route is. Getting this wrong is silent:
  // the id reads as undefined and the note records a ticket called "undefined".
  const res = await api<{ id: string }>(token, '/tickets', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(body),
  })
  return res.id
}

/**
 * Ask a human about a passage.
 *
 * A question hangs off a TICKET — that is the store's model, not an accident of
 * this page: a decision nobody can route to a piece of work is a decision that
 * never comes back. So asking from the margin of a document files the passage as
 * a ticket first and routes the question against it, `advisory` so it records a
 * decision without parking work nobody has claimed.
 */
export function createQuestion(
  token: string,
  body: { ticket: string; kind: string; mode: string; title: string; body?: string },
): Promise<{ id: string }> {
  return api<{ id: string }>(token, '/questions', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(body),
  })
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
