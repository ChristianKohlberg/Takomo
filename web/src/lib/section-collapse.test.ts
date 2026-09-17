import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { createNode } from './mindmap-crdt'
import { sectionSummaries, setSectionSummary, removeSectionSummary } from './section-collapse'

describe('authored section summaries', () => {
  it('requires a bounded nonempty summary and persists opt-in and removal to peers', () => {
    const doc = new Y.Doc(), peer = new Y.Doc()
    try {
      const id = createNode(doc, { parent: null, title: 'Architecture', by: 'Ada' })!
      expect(sectionSummaries(doc)).toEqual({})
      expect(setSectionSummary(doc, id, '   ')).toBe(false)
      expect(setSectionSummary(doc, id, 'x'.repeat(1001))).toBe(false)
      expect(setSectionSummary(doc, 'missing', 'Summary')).toBe(false)
      expect(setSectionSummary(doc, id, '  Components and responsibilities.  ')).toBe(true)
      Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
      expect(sectionSummaries(peer)).toEqual({ [id]: 'Components and responsibilities.' })
      expect(removeSectionSummary(doc, id)).toBe(true)
      Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc, Y.encodeStateVector(peer)))
      expect(sectionSummaries(peer)).toEqual({})
    } finally { doc.destroy(); peer.destroy() }
  })
})
