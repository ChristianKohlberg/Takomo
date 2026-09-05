// The /v1/mindmaps surface — brainstorming, before any of it is an idea.
//
// Two shapes are worth knowing before reading the calls. The detail read returns
// the map AND every node, because a canvas cannot draw half a tree — affordable
// because a map is capped at 500 nodes. And adding nodes takes a batch, because
// that is what an agent adding a branch sends; a single node is the same call
// with one entry.
//
// See docs/mindmaps.md for what a mindmap deliberately is not.
import { api } from './api'

export type MindmapStatus = 'open' | 'parked' | 'distilled'

export const MINDMAP_STATUSES: readonly MindmapStatus[] = [
  'open',
  'parked',
  'distilled',
] as const

/** A node is a sentence or two. The cap is the method — see the store module. */
export const MAX_NODE_TEXT = 280

export interface Mindmap {
  id: string
  project: string
  /** The root: what the map is about. */
  title: string
  summary?: string
  status: MindmapStatus
  metadata?: Record<string, unknown>
  /** How many thoughts hang off it. Derived server-side on every read. */
  nodes: number
  created_by?: string
  created_at: string
  updated_at?: string
  version?: number
}

/** What a branch became once it graduated. */
export interface Promoted {
  kind: 'epic' | 'initiative'
  id: string
}

export interface MindmapNode {
  id: string
  mindmap: string
  /** null = a first-ring branch off the root. */
  parent: string | null
  text: string
  position: number
  /**
   * Hand placement, or null to let the layout place it. One field rather than
   * two nullable numbers, because half a coordinate places nothing.
   */
  at: { x: number; y: number } | null
  promoted: Promoted | null
  created_by?: string
  created_at: string
  updated_at?: string
}

export interface MindmapDetail {
  mindmap: Mindmap
  nodes: MindmapNode[]
  /** Where each section stands, keyed by node id. See `plan-trace.ts`. */
  standing: PlanStanding
  total: number
}

// ---- the plan's history ---------------------------------------------------

/**
 * When a section last changed, and when anybody last agreed with it.
 *
 * A reading rather than a stored flag: a section confirmed BEFORE its last edit
 * is not confirmed any more, and no boolean can express that. The server groups
 * it in one query because a plan is drawn all at once.
 */
export interface StandingRow {
  /** ISO, or null when nothing has changed it since the trace began. */
  changed_at: string | null
  /** ISO, or null when nobody has ever said they read it. */
  reviewed_at: string | null
  /** Somebody agreed AFTER the last change. */
  confirmed: boolean
}

export type PlanStanding = Record<string, StandingRow>

/** What can happen to a section. A closed set, and small on purpose: a
 *  vocabulary anybody can hold in their head is what makes a history readable. */
export const TRACE_KINDS = [
  'authored',
  'renamed',
  'edited',
  'moved',
  'pruned',
  'reviewed',
  'proposed',
  'accepted',
  'rejected',
] as const

export type TraceKind = (typeof TRACE_KINDS)[number]

/**
 * The kinds a client may write.
 *
 * The rest are recorded by the paths that perform them, so nobody can claim to
 * have moved a node they did not move. These four are here because the document
 * view is where they happen: prose is edited over the sync socket, which the
 * server never sees as a request; a review is somebody saying so; and accepting
 * or rejecting a proposal is a decision the BROWSER carries out, because
 * markdown becomes nodes in the editor's own schema and only the editor has it.
 * Mirrors `CLIENT_TRACE_KINDS` in `src/store/trace.rs`.
 */
export const CLIENT_TRACE_KINDS = ['edited', 'reviewed', 'accepted', 'rejected'] as const

export type ClientTraceKind = (typeof CLIENT_TRACE_KINDS)[number]

export interface TraceEntry {
  id: string
  /** The section, or null for an act against the plan as a whole. */
  node: string | null
  kind: TraceKind
  /** The free-form thing the credential carried. */
  actor: string
  /** The person behind it, where the credential was bound to one. */
  user: string | null
  note: string | null
  /**
   * What the section said at that moment, or null when the act did not change
   * its prose.
   *
   * Kept because the CRDT update log cannot answer "what did 2.1 say last
   * Tuesday" — compaction rewrites it into one blob by design. A diff is two of
   * these. Read from the replica server-side and never taken from the caller: a
   * history somebody can write is not a history.
   */
  text: string | null
  at: string
}

export interface TracePage {
  items: TraceEntry[]
  total: number
  limit: number
  note?: string
}

/** The plan's history, newest first. One request for the whole plan: a view that
 *  draws every section would otherwise open with one request per section. */
export function getTrace(
  token: string,
  id: string,
  filter: { node?: string; limit?: number } = {},
): Promise<TracePage> {
  const p = new URLSearchParams()
  if (filter.node) p.set('node', filter.node)
  if (filter.limit) p.set('limit', String(filter.limit))
  const qs = p.toString()
  return api<TracePage>(token, `/mindmaps/${encodeURIComponent(id)}/trace${qs ? `?${qs}` : ''}`)
}

/** Record an act the server cannot see for itself. */
export function recordTrace(
  token: string,
  id: string,
  entry: { kind: ClientTraceKind; node?: string; note?: string },
): Promise<unknown> {
  return api(token, `/mindmaps/${encodeURIComponent(id)}/trace`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(entry),
  })
}

export interface MindmapsPage {
  items: Mindmap[]
  total: number
  limit: number
  note?: string
}

const json = { 'Content-Type': 'application/json' }

export function listMindmaps(
  token: string,
  filter: { project?: string; status?: string; q?: string; limit?: number } = {},
): Promise<MindmapsPage> {
  const p = new URLSearchParams()
  if (filter.project) p.set('project', filter.project)
  if (filter.status) p.set('status', filter.status)
  if (filter.q) p.set('q', filter.q)
  if (filter.limit) p.set('limit', String(filter.limit))
  const qs = p.toString()
  return api<MindmapsPage>(token, `/mindmaps${qs ? `?${qs}` : ''}`)
}

export function getMindmap(token: string, id: string): Promise<MindmapDetail> {
  return api<MindmapDetail>(token, `/mindmaps/${encodeURIComponent(id)}`)
}

export function createMindmap(
  token: string,
  body: { project: string; title: string; summary?: string },
): Promise<{ mindmap: Mindmap }> {
  return api<{ mindmap: Mindmap }>(token, '/mindmaps', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(body),
  })
}

export function patchMindmap(
  token: string,
  id: string,
  patch: { title?: string; summary?: string; status?: MindmapStatus },
): Promise<{ mindmap: Mindmap }> {
  return api<{ mindmap: Mindmap }>(token, `/mindmaps/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(patch),
  })
}

/** Throw a map away. Ordinary — see the module note. */
export function deleteMindmap(token: string, id: string): Promise<unknown> {
  return api(token, `/mindmaps/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

export interface NodeAdd {
  parent?: string | null
  text: string
  /** Omitted appends to the end of the ring; a value splits into it. */
  position?: number
}

/**
 * Add thoughts. Always sent as a batch — one node is a batch of one, and having
 * a single shape here means the page never has to decide which call to make.
 */
export function addNodes(
  token: string,
  id: string,
  nodes: NodeAdd[],
): Promise<{ nodes: MindmapNode[] }> {
  return api<{ nodes: MindmapNode[] }>(token, `/mindmaps/${encodeURIComponent(id)}/nodes`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify({
      nodes: nodes.map((n) => ({
        // `parent: null` and an absent parent mean the same thing on the way in
        // (a first-ring branch), so the null is dropped rather than sent.
        ...(n.parent ? { parent: n.parent } : {}),
        text: n.text,
        ...(n.position !== undefined ? { position: n.position } : {}),
      })),
    }),
  })
}

export interface NodePatch {
  text?: string
  /** null lifts the node to the first ring; absent leaves it where it is. */
  parent?: string | null
  position?: number
  /** null hands the node back to the layout — what "tidy up" sends. */
  at?: { x: number; y: number } | null
}

export function patchNode(
  token: string,
  id: string,
  node: string,
  patch: NodePatch,
): Promise<{ node: MindmapNode }> {
  return api<{ node: MindmapNode }>(
    token,
    `/mindmaps/${encodeURIComponent(id)}/nodes/${encodeURIComponent(node)}`,
    { method: 'PATCH', headers: json, body: JSON.stringify(patch) },
  )
}

export function deleteNode(token: string, id: string, node: string): Promise<unknown> {
  return api(
    token,
    `/mindmaps/${encodeURIComponent(id)}/nodes/${encodeURIComponent(node)}`,
    { method: 'DELETE' },
  )
}

/**
 * Graduate a branch. The node stays and keeps a link to what it became — the map
 * is the record of how the thinking got there.
 */
export function promoteNode(
  token: string,
  id: string,
  node: string,
  target: 'epic' | 'initiative',
): Promise<{ node: MindmapNode; created: Promoted & { children?: string[] } }> {
  return api(
    token,
    `/mindmaps/${encodeURIComponent(id)}/nodes/${encodeURIComponent(node)}/promote`,
    { method: 'POST', headers: json, body: JSON.stringify({ target }) },
  )
}

// ---- the sync socket ------------------------------------------------------

/**
 * A ticket for one map's sync socket.
 *
 * The same `tkd_` credential `/documents` mints, widened from "document session"
 * to "collab session": one object, expiring, revocable, and never more
 * permissive than the token that asked for it. It exists because a browser
 * `WebSocket` cannot set an `Authorization` header — the same limitation that
 * keeps `/board` polling `/v1/events` — so the credential has to ride the
 * handshake, and putting the viewer's real token in a query string would land it
 * in every access log on the path.
 */
export interface MindmapSession {
  actor?: string
  durability_ack?: boolean
  object: string
  kind: 'mindmap'
  mindmap: string
  session: string
  /** The `tkd_` ticket. Shown once; the server keeps only its hash. */
  token: string
  /** False when the minting token had no `write` scope. */
  can_write: boolean
  /** The name collaborators see against this cursor. */
  display: string
  expires_at: string
  /** The socket BASE, without the room. See `mindmapSyncBase`. */
  url: string
  /** The room to append — the map id. */
  room: string
  note?: string
}

export function mintMindmapSession(token: string, id: string): Promise<MindmapSession> {
  return api<MindmapSession>(token, `/mindmaps/${encodeURIComponent(id)}/session`, {
    method: 'POST',
  })
}

/**
 * The absolute `ws(s)://` BASE for the sync socket — without the room.
 *
 * Returning the base rather than a finished URL is a wire requirement, not
 * fussiness: `y-websocket` composes its own address as
 * `serverUrl + "/" + room + "?" + params`, so a complete URL handed to it comes
 * back mangled, with the room after the query string. Same rule, same reason, as
 * `syncBase` in `documents.ts`.
 */
export function mindmapSyncBase(session: MindmapSession): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}${session.url}`
}
