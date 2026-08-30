// The mindmap document: reading and writing the Y.Doc itself.
//
// The shape, the caps and the deterministic read rules live next door in
// `mindmap-doc.ts`, which imports nothing. That split is not tidiness: this file
// pulls in Yjs, and Yjs is the largest dependency the app has after the editor.
// Keeping the model pure means a component can name a `MapNode` or a `NodeKind`
// with a type-only import and pay nothing for it, and it means the normalisation
// rules are tested without a document at all.
//
// Everything below writes through `doc.transact`, so one gesture is one update on
// the wire rather than a burst a collaborator watches arrive in pieces.
import * as Y from 'yjs'

import {
  ATTACHMENT_KINDS,
  MAX_ATTACHMENTS,
  MAX_ATTACHMENT_GIST,
  MAX_ATTACHMENT_NAME,
  MAX_ATTACHMENT_REF,
  MAX_EDGE_LABEL,
  MAX_REL_LABEL,
  MAX_ICONS,
  MAX_NODES,
  MAX_NOTES,
  MAX_RELATIONSHIPS,
  MAX_TITLE,
  NODE_KINDS,
  descendantsOf,
  normaliseNodes,
  normaliseRelationships,
  orderBetween,
  type Attachment,
  type AttachmentKind,
  type MapNode,
  type NodeFields,
  type NodeKind,
  type RawNode,
  type Relationship,
} from './mindmap-doc'
import type { Point } from './mindmap-layout'

// ---- reading the document -------------------------------------------------

type Inner = Y.Map<unknown>

export function nodesMap(doc: Y.Doc): Y.Map<Inner> {
  return doc.getMap<Inner>('nodes')
}

export function relationshipsMap(doc: Y.Doc): Y.Map<Inner> {
  return doc.getMap<Inner>('relationships')
}

function readText(m: Inner, key: string): string {
  const value = m.get(key)
  if (value instanceof Y.Text) return value.toString()
  return typeof value === 'string' ? value : ''
}

function readString(m: Inner, key: string, fallback = ''): string {
  const value = m.get(key)
  return typeof value === 'string' ? value : fallback
}

/**
 * A timestamp, as epoch milliseconds.
 *
 * The server writes an i64 and reads one back; this used to write an ISO string,
 * so each side read the other's value as a fallback — every node created in the
 * canvas came back over the API dated 1970, and touching an API-created node in
 * the UI reset its timestamps to the same. One representation, and it is the
 * server's, because that is what the wire format already promises.
 */
function readTimestamp(m: Inner, key: string): number {
  const value = m.get(key)
  if (typeof value === 'number' && Number.isFinite(value)) return value
  // Tolerated on read only, for anything written before this was settled.
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    if (!Number.isNaN(parsed)) return parsed
  }
  return 0
}

function readNumber(m: Inner, key: string): number | null {
  const value = m.get(key)
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function readAttachments(m: Inner): Attachment[] {
  const value = m.get('attachments')
  if (!(value instanceof Y.Map)) return []
  const out: Attachment[] = []
  value.forEach((entry, id) => {
    if (!(entry instanceof Y.Map)) return
    const kind = readString(entry, 'kind', 'link')
    out.push({
      id,
      kind: (ATTACHMENT_KINDS as readonly string[]).includes(kind)
        ? (kind as AttachmentKind)
        : 'link',
      name: readString(entry, 'name'),
      gist: readString(entry, 'gist'),
      ref: readString(entry, 'ref'),
    })
  })
  // By name then id, matching `attachments_of` in src/store/mindmapdoc.rs and
  // what the spec documents. A Y.Map has no order of its own, so if the two
  // sides sorted differently the list would depend on who was looking.
  out.sort((a, b) =>
    a.name < b.name ? -1 : a.name > b.name ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  )
  return out
}

/** One node, as the document holds it. Every field defaulted, because a peer may
 *  have written a partial record and a missing key must not blank the canvas. */
export function readRawNode(id: string, m: Inner): RawNode {
  const parent = m.get('parent')
  const x = readNumber(m, 'x')
  const y = readNumber(m, 'y')
  const kind = readString(m, 'kind', 'thought')
  const origin = readString(m, 'origin', 'human')
  const icons = m.get('icons')
  const promotedKind = readString(m, 'promoted_kind')
  const promotedId = readString(m, 'promoted_id')
  return {
    id,
    parent: typeof parent === 'string' && parent.length > 0 ? parent : null,
    order: readString(m, 'order'),
    title: readText(m, 'title'),
    notes: readText(m, 'notes'),
    at: x !== null && y !== null ? { x, y } : null,
    edge_label: readString(m, 'edge_label'),
    kind: (NODE_KINDS as readonly string[]).includes(kind) ? (kind as NodeKind) : 'thought',
    origin: origin === 'agent' ? 'agent' : 'human',
    reviewed: m.get('reviewed') === true,
    icons:
      icons instanceof Y.Array
        ? (icons.toArray().filter((i) => typeof i === 'string') as string[]).slice(0, MAX_ICONS)
        : [],
    color: readString(m, 'color'),
    shape: readString(m, 'shape'),
    attachments: readAttachments(m),
    promoted:
      (promotedKind === 'epic' || promotedKind === 'initiative') && promotedId
        ? { kind: promotedKind, id: promotedId }
        : null,
    created_by: readString(m, 'created_by'),
    created_at: readTimestamp(m, 'created_at'),
    updated_at: readTimestamp(m, 'updated_at'),
  }
}

/** The whole tree, repaired and ranked. */
export function readNodes(doc: Y.Doc): MapNode[] {
  const raw: RawNode[] = []
  nodesMap(doc).forEach((m, id) => {
    if (m instanceof Y.Map) raw.push(readRawNode(id, m))
  })
  return normaliseNodes(raw)
}

export function readRelationships(doc: Y.Doc, nodes: readonly MapNode[]): Relationship[] {
  const raw: Relationship[] = []
  relationshipsMap(doc).forEach((m, id) => {
    if (!(m instanceof Y.Map)) return
    raw.push({
      id,
      from: readString(m, 'from'),
      to: readString(m, 'to'),
      label: readString(m, 'label'),
    })
  })
  return normaliseRelationships(raw, nodes)
}

// ---- writing --------------------------------------------------------------

let minted = 0

/** Ids are minted in the browser because nodes are created at typing speed. They
 *  are opaque and only have to be unique; the `mn-` prefix matches the server's. */
function mintId(prefix: string): string {
  minted += 1
  return `${prefix}-${Date.now().toString(36)}${minted.toString(36)}${Math.random()
    .toString(36)
    .slice(2, 8)}`
}

function now(): number {
  return Date.now()
}

/**
 * Replace a Y.Text's content with `next`, touching only what actually changed.
 *
 * A delete-all-then-insert would work and would be wrong: it discards the
 * character positions somebody else's caret is anchored to, so their cursor jumps
 * to the start of the box every time you type. Trimming the common prefix and
 * suffix keeps the edit as small as the change.
 */
export function applyText(target: Y.Text, next: string): void {
  const current = target.toString()
  if (current === next) return
  let prefix = 0
  while (prefix < current.length && prefix < next.length && current[prefix] === next[prefix]) {
    prefix += 1
  }
  let suffix = 0
  while (
    suffix < current.length - prefix &&
    suffix < next.length - prefix &&
    current[current.length - 1 - suffix] === next[next.length - 1 - suffix]
  ) {
    suffix += 1
  }
  const removed = current.length - prefix - suffix
  if (removed > 0) target.delete(prefix, removed)
  const added = next.slice(prefix, next.length - suffix)
  if (added.length > 0) target.insert(prefix, added)
}

function ytext(m: Inner, key: string): Y.Text {
  const existing = m.get(key)
  if (existing instanceof Y.Text) return existing
  m.set(key, new Y.Text())
  return m.get(key) as Y.Text
}

function node(doc: Y.Doc, id: string): Inner | null {
  const m = nodesMap(doc).get(id)
  return m instanceof Y.Map ? m : null
}

function touch(m: Inner): void {
  m.set('updated_at', now())
}

export interface CreateNode {
  parent: string | null
  /** The sibling this one goes after; null appends to the end of the ring. */
  after?: string | null
  title: string
  by: string
}

/** Add a thought. Returns its id, or null when the map is already at its cap. */
export function createNode(doc: Y.Doc, opts: CreateNode): string | null {
  const nodes = readNodes(doc)
  if (nodes.length >= MAX_NODES) return null
  const siblings = nodes.filter((n) => n.parent === (opts.parent ?? null))
  const order = orderBetween(siblings, opts.after ?? null)
  const id = mintId('mn')
  const stamp = now()
  doc.transact(() => {
    const map = nodesMap(doc)
    map.set(id, new Y.Map())
    const m = map.get(id) as Inner
    m.set('parent', opts.parent)
    m.set('order', order)
    m.set('title', new Y.Text())
    m.set('notes', new Y.Text())
    m.set('x', null)
    m.set('y', null)
    m.set('edge_label', '')
    m.set('kind', 'thought')
    m.set('origin', 'human')
    m.set('reviewed', false)
    m.set('icons', new Y.Array())
    m.set('color', '')
    m.set('shape', '')
    m.set('attachments', new Y.Map())
    m.set('promoted_kind', null)
    m.set('promoted_id', null)
    m.set('created_by', opts.by)
    m.set('created_at', stamp)
    m.set('updated_at', stamp)
    applyText(ytext(m, 'title'), opts.title.slice(0, MAX_TITLE))
  })
  return id
}

/** Remove a node and everything under it, plus every relationship that touched
 *  any of them — a relationship to a node that is gone is not an edge. */
export function deleteSubtree(doc: Y.Doc, id: string): void {
  const nodes = readNodes(doc)
  const doomed = new Set([id, ...descendantsOf(nodes, id)])
  doc.transact(() => {
    const map = nodesMap(doc)
    for (const victim of doomed) map.delete(victim)
    const rels = relationshipsMap(doc)
    const dead: string[] = []
    rels.forEach((m, relId) => {
      if (!(m instanceof Y.Map)) return
      if (doomed.has(readString(m, 'from')) || doomed.has(readString(m, 'to'))) dead.push(relId)
    })
    for (const relId of dead) rels.delete(relId)
  })
}

export function setTitle(doc: Y.Doc, id: string, title: string): void {
  const m = node(doc, id)
  if (!m) return
  doc.transact(() => {
    applyText(ytext(m, 'title'), title.slice(0, MAX_TITLE))
    touch(m)
  })
}

export function setNotes(doc: Y.Doc, id: string, notes: string): void {
  const m = node(doc, id)
  if (!m) return
  doc.transact(() => {
    applyText(ytext(m, 'notes'), notes.slice(0, MAX_NOTES))
    touch(m)
  })
}

/**
 * The length caps that apply to a field written here.
 *
 * The server refuses these values on its own routes; the editor has to refuse
 * them too, because a browser writes the document directly and the server never
 * sees the keystroke. Without this a label typed on the canvas syncs happily and
 * the identical label sent over REST comes back 422.
 */
const FIELD_CAPS: Partial<Record<keyof NodeFields, number>> = {
  edge_label: MAX_EDGE_LABEL,
}

export function setFields(doc: Y.Doc, id: string, fields: Partial<NodeFields>): void {
  const m = node(doc, id)
  if (!m) return
  doc.transact(() => {
    for (const [key, value] of Object.entries(fields)) {
      if (value === undefined) continue
      const cap = FIELD_CAPS[key as keyof NodeFields]
      m.set(key, cap !== undefined && typeof value === 'string' ? value.slice(0, cap) : value)
    }
    touch(m)
  })
}

/** Drop a node under a new parent. It un-pins, because a dropped node is placed
 *  by the layout under where it landed, and it goes to the end of that ring. */
export function reparent(doc: Y.Doc, id: string, parent: string | null): void {
  const m = node(doc, id)
  if (!m) return
  const nodes = readNodes(doc)
  const order = orderBetween(
    nodes.filter((n) => n.parent === parent && n.id !== id),
    null,
  )
  doc.transact(() => {
    m.set('parent', parent)
    m.set('order', order)
    m.set('x', null)
    m.set('y', null)
    touch(m)
  })
}

export function place(doc: Y.Doc, id: string, at: Point | null): void {
  const m = node(doc, id)
  if (!m) return
  doc.transact(() => {
    m.set('x', at ? at.x : null)
    m.set('y', at ? at.y : null)
    touch(m)
  })
}

/** Hand every node back to the layout. */
export function tidyAll(doc: Y.Doc): void {
  doc.transact(() => {
    nodesMap(doc).forEach((m) => {
      if (!(m instanceof Y.Map)) return
      if (m.get('x') === null && m.get('y') === null) return
      m.set('x', null)
      m.set('y', null)
    })
  })
}

// ---- relationships --------------------------------------------------------

/** A labelled edge outside the hierarchy. Returns its id, or null at the cap. */
export function createRelationship(
  doc: Y.Doc,
  from: string,
  to: string,
  label: string,
): string | null {
  if (from === to) return null
  const rels = relationshipsMap(doc)
  if (rels.size >= MAX_RELATIONSHIPS) return null
  const id = mintId('mr')
  doc.transact(() => {
    rels.set(id, new Y.Map())
    const m = rels.get(id) as Inner
    m.set('from', from)
    m.set('to', to)
    m.set('label', label.slice(0, MAX_REL_LABEL))
  })
  return id
}

export function deleteRelationship(doc: Y.Doc, id: string): void {
  doc.transact(() => relationshipsMap(doc).delete(id))
}

// ---- attachments ----------------------------------------------------------

export type AttachmentDraft = Omit<Attachment, 'id'>

/** Add a pointer to the selected node. Returns null when the node is at its cap. */
export function addAttachment(
  doc: Y.Doc,
  nodeId: string,
  draft: AttachmentDraft,
): string | null {
  const m = node(doc, nodeId)
  if (!m) return null
  const id = mintId('ma')
  let refused = false
  doc.transact(() => {
    const existing = m.get('attachments')
    if (!(existing instanceof Y.Map)) m.set('attachments', new Y.Map())
    const bag = m.get('attachments') as Y.Map<unknown>
    if (bag.size >= MAX_ATTACHMENTS) {
      refused = true
      return
    }
    bag.set(id, new Y.Map())
    const entry = bag.get(id) as Inner
    entry.set('kind', draft.kind)
    entry.set('name', draft.name.slice(0, MAX_ATTACHMENT_NAME))
    entry.set('gist', draft.gist.slice(0, MAX_ATTACHMENT_GIST))
    entry.set('ref', draft.ref.slice(0, MAX_ATTACHMENT_REF))
    touch(m)
  })
  return refused ? null : id
}

export function removeAttachment(doc: Y.Doc, nodeId: string, attachmentId: string): void {
  const m = node(doc, nodeId)
  if (!m) return
  doc.transact(() => {
    const bag = m.get('attachments')
    if (bag instanceof Y.Map) bag.delete(attachmentId)
    touch(m)
  })
}
