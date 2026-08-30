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
  total: number
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
