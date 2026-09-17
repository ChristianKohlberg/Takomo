import type * as Y from 'yjs'
import { nodesMap } from './mindmap-crdt'

/** A nonempty, authored summary opts a section into folding. Shared with the document. */
export const SECTION_SUMMARY = 'collapse_summary'
export function sectionSummaries(doc: Y.Doc): Record<string, string> {
  const summaries: Record<string, string> = {}
  for (const [id, node] of nodesMap(doc)) {
    const summary = node.get(SECTION_SUMMARY)
    if (typeof summary === 'string' && summary.trim()) summaries[id] = summary
  }
  return summaries
}
export function setSectionSummary(doc: Y.Doc, id: string, summary: string): boolean {
  const node = nodesMap(doc).get(id)
  if (!node || !summary.trim() || summary.length > 1000) return false
  doc.transact(() => node.set(SECTION_SUMMARY, summary.trim()))
  return true
}
export function removeSectionSummary(doc: Y.Doc, id: string): boolean {
  const node = nodesMap(doc).get(id)
  if (!node) return false
  doc.transact(() => node.delete(SECTION_SUMMARY))
  return true
}
