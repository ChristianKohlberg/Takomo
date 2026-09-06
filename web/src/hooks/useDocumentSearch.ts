import { useEffect, useMemo, useState } from 'react'
import type * as Y from 'yjs'
import { findDocumentMatches } from '@/lib/document-search'

export function useDocumentSearch(doc: Y.Doc | null, nodes: readonly { id: string; title: string }[]) {
  const [query, updateQuery] = useState('')
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [revision, setRevision] = useState(0)
  useEffect(() => {
    if (!doc || !query) return
    const refresh = (transaction: Y.Transaction) => {
      if (transaction.changed.size) setRevision(value => value + 1)
    }
    doc.on('afterTransaction', refresh)
    return () => { doc.off('afterTransaction', refresh) }
  }, [doc, query])
  const matches = useMemo(() => {
    // Y.Doc is mutable; a committed transaction invalidates this projection.
    void revision
    return doc ? findDocumentMatches(nodes, doc, query) : []
  }, [doc, nodes, query, revision])
  const activeIndex = matches.findIndex(match => match.key === activeKey)
  // A removed match never silently selects an unrelated section.
  const activeMatch = activeIndex < 0 ? null : matches[activeIndex] ?? null
  const move = (direction: 1 | -1) => {
    if (!matches.length) return
    const index = activeIndex < 0 ? (direction === 1 ? 0 : matches.length - 1) : (activeIndex + direction + matches.length) % matches.length
    setActiveKey(matches[index]?.key ?? null)
  }
  return {
    query, matches, activeIndex, activeMatch,
    setQuery(value: string) {
      updateQuery(value)
      setActiveKey(doc ? findDocumentMatches(nodes, doc, value)[0]?.key ?? null : null)
    },
    next: () => move(1), previous: () => move(-1),
    clear() { updateQuery(''); setActiveKey(null) },
  }
}
