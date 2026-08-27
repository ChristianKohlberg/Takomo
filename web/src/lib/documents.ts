// The `/documents` client: filing over HTTP, prose over a WebSocket.
//
// Every function here handles a document's title, folder and status. **None of
// them carries the text**, and there is no route that would: the prose is a Yjs
// CRDT synchronised over the sync socket, because a `PUT body` is
// last-write-wins and losing what somebody typed while an agent was thinking is
// the failure this whole surface exists to remove.
//
// So the interesting function is [`mintSession`], which is how the editor gets
// permission to open that socket at all.
import { api } from './api'

export type DocumentStatus = 'draft' | 'review' | 'settled' | 'archived'

export const DOCUMENT_STATUSES: readonly DocumentStatus[] = [
  'draft',
  'review',
  'settled',
  'archived',
] as const

export interface Doc {
  id: string
  project: string
  title: string
  /** Folder, `/`-separated. Empty is the top level. */
  path: string
  status: DocumentStatus
  /** The initiative this was distilled from, if any. */
  initiative: string | null
  metadata: unknown
  version: number
  created_by: string
  created_at: string
  updated_at: string
  archived_at: string | null
  /** Bytes of CRDT history. Derived server-side, never stored. */
  bytes: number
  /** Rows in the update log; high means a compaction is due. */
  updates: number
}

export interface Paged<T> {
  items: T[]
  total: number
  limit: number
  note?: string
}

const json = { 'Content-Type': 'application/json' }
const enc = encodeURIComponent

export function listDocuments(
  token: string,
  project: string,
  includeArchived = false,
): Promise<Paged<Doc>> {
  const tail = includeArchived ? '?archived=include' : ''
  return api<Paged<Doc>>(token, `/projects/${enc(project)}/documents${tail}`)
}

export interface DocumentFields {
  title: string
  path?: string
  status?: DocumentStatus
  initiative?: string | null
}

export function createDocument(
  token: string,
  project: string,
  f: DocumentFields,
): Promise<Doc> {
  return api<Doc>(token, `/projects/${enc(project)}/documents`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(f),
  })
}

export function patchDocument(
  token: string,
  id: string,
  f: Partial<DocumentFields>,
): Promise<Doc> {
  return api<Doc>(token, `/documents/${enc(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(f),
  })
}

export function archiveDocument(token: string, id: string): Promise<Doc> {
  return api<Doc>(token, `/documents/${enc(id)}`, { method: 'DELETE' })
}

export function unarchiveDocument(token: string, id: string): Promise<Doc> {
  return api<Doc>(token, `/documents/${enc(id)}/unarchive`, { method: 'POST' })
}

export interface DocSession {
  document: string
  session: string
  /** The `tkd_` ticket. Shown once; the server keeps only its hash. */
  token: string
  /** False when the minting token had no `write` scope. */
  can_write: boolean
  /** The name collaborators see next to this caret. */
  display: string
  expires_at: string
  /** The socket BASE, without the room. See `syncBase`. */
  url: string
  /** The room to append — the document id. */
  room: string
}

/**
 * Ask for a ticket to open one document's sync socket.
 *
 * A browser `WebSocket` cannot set an `Authorization` header — the same
 * limitation that keeps `/board` polling `/v1/events` instead of using the SSE
 * stream. Polling is not an option for a CRDT, so the credential has to ride the
 * handshake, and sending the viewer's real token in a query string would put it
 * in every access log on the path.
 *
 * The ticket that comes back reaches this one document, expires, and carries no
 * more than the token that asked for it.
 */
export function mintSession(token: string, id: string): Promise<DocSession> {
  return api<DocSession>(token, `/documents/${enc(id)}/session`, { method: 'POST' })
}

/**
 * The absolute `ws(s)://` BASE for the sync socket — without the room.
 *
 * Returning the base rather than a finished URL is not fussiness: `y-websocket`
 * composes its own address as `serverUrl + "/" + room + "?" + params`, so a
 * complete URL handed to it comes back mangled — the room lands after the query
 * string. The room and the ticket are passed separately, which is why the
 * server puts the document id in the last path segment.
 *
 * Derived from `location` rather than configured: the binary serves both the
 * page and `/v1`, so the socket is always same-origin — which is also what keeps
 * it inside the page's `connect-src 'self'` CSP.
 */
export function syncBase(session: DocSession): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}${session.url}`
}

/** A folder tree over the flat list, derived on every read and never stored. */
export interface Folder {
  path: string
  name: string
  children: Folder[]
  docs: Doc[]
}

/**
 * Group documents into folders by their `path`.
 *
 * A folder exists because a document names it — the same rule
 * `initiative-tree.ts` follows, which is why neither needs a folder table nor
 * has an orphaned-directory problem: the last document to leave takes the folder
 * with it.
 */
export function buildTree(docs: readonly Doc[]): Folder {
  const root: Folder = { path: '', name: '', children: [], docs: [] }
  const byPath = new Map<string, Folder>([['', root]])

  const ensure = (path: string): Folder => {
    const existing = byPath.get(path)
    if (existing) return existing
    const cut = path.lastIndexOf('/')
    const parentPath = cut === -1 ? '' : path.slice(0, cut)
    const name = cut === -1 ? path : path.slice(cut + 1)
    const folder: Folder = { path, name, children: [], docs: [] }
    byPath.set(path, folder)
    ensure(parentPath).children.push(folder)
    return folder
  }

  for (const doc of docs) ensure(doc.path).docs.push(doc)

  const sort = (f: Folder): Folder => {
    f.children.sort((a, b) => a.name.localeCompare(b.name))
    f.docs.sort((a, b) => a.title.localeCompare(b.title))
    f.children.forEach(sort)
    return f
  }
  return sort(root)
}

export interface RunResult {
  proposal: string
  document: string
  status: string
  summary: string
  operations: number
  skipped: string[]
  model: string
}

/**
 * Ask the server's document agent for a proposal.
 *
 * The ONE call in this app that reaches a language model, and it reaches it
 * through Takomo rather than from the browser — so no key ever lands in a page,
 * and the answer goes through the same validation a fleet agent's ops do.
 *
 * It returns a PROPOSAL, not an edit. Nothing is in the document when this
 * resolves; the review panel is where it becomes text.
 */
export function runAgent(
  token: string,
  id: string,
  instruction: string,
  scope?: string[],
): Promise<RunResult> {
  return api<RunResult>(token, `/documents/${enc(id)}/run`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify({ instruction, scope: scope?.length ? scope : null }),
  })
}
