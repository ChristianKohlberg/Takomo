import * as Y from 'yjs'
import type { Editor } from '@tiptap/react'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { absolutePositionToRelativePosition, relativePositionToAbsolutePosition, ySyncPluginKey } from '@tiptap/y-tiptap'

export const COMMENT_FIELD = 'documentComments'
export const MAX_COMMENT_LENGTH = 5000
export interface CommentAnchor { quote: string; start: Record<string, unknown>; end: Record<string, unknown> }
export interface CommentMessage { id: string; author: string; text: string; created: number }
export interface CommentThread { id: string; sectionId: string; anchor: CommentAnchor; resolved: boolean; messages: CommentMessage[] }

/** Bound to the selected CRDT text, never searched for elsewhere by quote. */
export function captureCommentAnchor(editor: Editor): CommentAnchor | null {
  const { from, to } = editor.state.selection
  const sync = ySyncPluginKey.getState(editor.state)
  if (from === to || !sync?.binding) return null
  const quote = editor.state.doc.textBetween(from, to, '\n')
  if (!quote.trim() || quote.length > MAX_COMMENT_LENGTH) return null
  const start = absolutePositionToRelativePosition(from, sync.type, sync.binding.mapping)
  const end = absolutePositionToRelativePosition(to, sync.type, sync.binding.mapping)
  return { quote, start: Y.relativePositionToJSON(start), end: Y.relativePositionToJSON(end) }
}

export function resolveCommentAnchor(editor: Editor, anchor: CommentAnchor, doc: ProseMirrorNode = editor.state.doc): { from: number; to: number } | null {
  const sync = ySyncPluginKey.getState(editor.state)
  if (!sync?.binding) return null
  try {
    const from = relativePositionToAbsolutePosition(sync.doc, sync.type, Y.createRelativePositionFromJSON(anchor.start), sync.binding.mapping)
    const to = relativePositionToAbsolutePosition(sync.doc, sync.type, Y.createRelativePositionFromJSON(anchor.end), sync.binding.mapping)
    if (from === null || to === null || from >= to || to > doc.content.size) return null
    // A replaced passage must not quietly adopt an old discussion. Edits outside
    // the range keep its CRDT anchors and highlight; changed quotes stay visible
    // in the panel with an explicit unattached state.
    if (doc.textBetween(from, to, '\n') !== anchor.quote) return null
    return { from, to }
  } catch { return null }
}

export function readCommentThreads(doc: Y.Doc, sectionId?: string): CommentThread[] {
  const out: CommentThread[] = []
  doc.getMap<Y.Map<unknown>>(COMMENT_FIELD).forEach((entry, id) => {
    if (!(entry instanceof Y.Map)) return
    const node = entry.get('sectionId')
    const anchor = entry.get('anchor') as CommentAnchor | undefined
    const replies = entry.get('messages')
    if (typeof node !== 'string' || (sectionId !== undefined && node !== sectionId) || !anchor || typeof anchor.quote !== 'string' || !anchor.start || !anchor.end || !(replies instanceof Y.Map)) return
    const messages: CommentMessage[] = []
    replies.forEach((value, key) => {
      const m = value as CommentMessage | undefined
      if (m && typeof m.text === 'string' && typeof m.author === 'string' && typeof m.created === 'number') messages.push({ ...m, id: key })
    })
    messages.sort((a, b) => a.created - b.created || a.id.localeCompare(b.id))
    out.push({ id, sectionId: node, anchor, resolved: entry.get('resolved') === true, messages })
  })
  return out.sort((a, b) => Number(a.resolved) - Number(b.resolved) || (a.messages[0]?.created ?? 0) - (b.messages[0]?.created ?? 0) || a.id.localeCompare(b.id))
}

function message(author: string, text: string): CommentMessage {
  const trimmed = text.trim()
  if (!trimmed || trimmed.length > MAX_COMMENT_LENGTH) throw new Error('invalid-comment')
  return { id: crypto.randomUUID(), author, text: trimmed, created: Date.now() }
}
export function createCommentThread(doc: Y.Doc, sectionId: string, anchor: CommentAnchor, author: string, text: string): string {
  const first = message(author, text)
  const id = crypto.randomUUID()
  doc.transact(() => {
    const entry = new Y.Map<unknown>()
    doc.getMap<Y.Map<unknown>>(COMMENT_FIELD).set(id, entry)
    entry.set('sectionId', sectionId)
    entry.set('anchor', anchor)
    entry.set('resolved', false)
    const messages = new Y.Map<CommentMessage>()
    entry.set('messages', messages)
    messages.set(first.id, first)
  }, 'document-comment')
  return id
}
export function replyToComment(doc: Y.Doc, id: string, author: string, text: string): void {
  const reply = message(author, text)
  const messages = doc.getMap<Y.Map<unknown>>(COMMENT_FIELD).get(id)?.get('messages')
  if (!(messages instanceof Y.Map)) throw new Error('missing-comment')
  messages.set(reply.id, reply)
}
export function resolveCommentThread(doc: Y.Doc, id: string, resolved: boolean): void {
  const entry = doc.getMap<Y.Map<unknown>>(COMMENT_FIELD).get(id)
  if (!entry) throw new Error('missing-comment')
  entry.set('resolved', resolved)
}
