import * as Y from 'yjs'
import type { Node as PMNode } from '@tiptap/pm/model'
import { readProseOf } from './mindmap-crdt'

export interface TextMatch { from: number; to: number }
export interface DocumentMatch extends TextMatch {
  sectionId: string
  kind: 'heading' | 'prose'
  key: string
}

/** Escaping makes punctuation literal. Unicode regex preserves original UTF-16
 * offsets even when case conversion would change string length (e.g. İ). */
export function literalMatches(text: string, query: string, offset = 0): TextMatch[] {
  if (!query) return []
  const expression = new RegExp(query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'giu')
  return Array.from(text.matchAll(expression), match => ({ from: offset + match.index, to: offset + match.index + match[0].length }))
}

/** Text blocks stay separate so a result cannot cross unrelated paragraphs or cells. */
export function proseMatches(doc: PMNode, query: string): TextMatch[] {
  const found: TextMatch[] = []
  doc.descendants((node, position) => {
    if (!node.isTextblock) return true
    found.push(...literalMatches(node.textBetween(0, node.content.size, '', '\n'), query, position + 1))
    return false
  })
  return found
}

const textBlocks = new Set(['paragraph', 'heading', 'codeBlock'])
const leaves = new Set(['hardBreak', 'horizontalRule', 'image'])

/** Mirror ProseMirror positions directly from the shared fragment: no editor
 * needs to mount, and reading never initializes or writes a missing fragment. */
export function fragmentMatches(fragment: Y.XmlFragment, query: string): TextMatch[] {
  const found: TextMatch[] = []
  const walk = (part: Y.XmlElement | Y.XmlText | Y.XmlHook, position: number): number => {
    if (part instanceof Y.XmlText) return part.length
    if (!(part instanceof Y.XmlElement)) return 1
    if (leaves.has(part.nodeName)) return 1
    if (textBlocks.has(part.nodeName)) {
      const text = part.toArray().map(child => child instanceof Y.XmlText
        ? child.toDelta().map((delta: { insert: unknown }) => typeof delta.insert === 'string' ? delta.insert : '\n').join('')
        : '\n').join('')
      found.push(...literalMatches(text, query, position + 1))
      return text.length + 2
    }
    let size = 2
    for (const child of part.toArray()) size += walk(child, position + size - 1)
    return size
  }
  let position = 0
  for (const child of fragment.toArray()) position += walk(child, position)
  return found
}

export function findDocumentMatches(
  nodes: readonly { id: string; title: string }[], doc: Y.Doc, query: string,
): DocumentMatch[] {
  if (!query) return []
  return nodes.flatMap(node => {
    // A stale tree projection must never return results for a deleted section.
    if (!doc.getMap('nodes').has(node.id)) return []
    const fragment = readProseOf(doc, node.id)
    const heading = literalMatches(node.title, query).map(match => ({ ...match, kind: 'heading' as const }))
    const prose = fragment ? fragmentMatches(fragment, query).map(match => ({ ...match, kind: 'prose' as const })) : []
    return [...heading, ...prose].map(match => ({ ...match, sectionId: node.id, key: `${node.id}:${match.kind}:${match.kind === 'heading' ? `${node.title}:${match.from}:${match.to}` : fragment ? fragmentAnchor(fragment, match.from) : match.from}` }))
  })
}

/** Anchor result identity to CRDT characters, so remote insertions above a match
 * move the same result and deleting it cannot activate the next occurrence. */
function fragmentAnchor(fragment: Y.XmlFragment, target: number): string {
  let anchor = ''
  const walk = (part: Y.XmlElement | Y.XmlText | Y.XmlHook, position: number): number => {
    if (part instanceof Y.XmlText) {
      if (position <= target && target < position + part.length) {
        anchor = JSON.stringify(Y.createRelativePositionFromTypeIndex(part, target - position))
      }
      return part.length
    }
    if (!(part instanceof Y.XmlElement) || leaves.has(part.nodeName)) return 1
    let size = 2
    for (const child of part.toArray()) size += walk(child, position + size - 1)
    return size
  }
  let position = 0
  for (const child of fragment.toArray()) position += walk(child, position)
  return anchor || String(target)
}
