import * as Y from 'yjs'
import { nodesMap, readPlanTree } from './mindmap-crdt'
import { flattenSections, planSections } from './plan-sections'

export interface ReferenceSection { key: string; title: string; number: string }
const indexes = new WeakMap<Y.Doc, ReturnType<typeof createIndex>>()

function createIndex(doc: Y.Doc) {
  let snapshot: ReferenceSection[] | null = null
  const listeners = new Set<() => void>()
  const nodes = nodesMap(doc)
  // One observer and one shape-only rebuild for every chip/picker in a document.
  // Prose/review changes do not invalidate numbering or walk the document text.
  nodes.observeDeep(events => {
    if (!events.some(event => event.target === nodes || event.path[1] === 'title' ||
      (event.path.length === 1 && event instanceof Y.YMapEvent &&
        [...event.keysChanged].some(key => ['title', 'parent', 'order'].includes(key))))) return
    snapshot = null
    listeners.forEach(notify => notify())
  })
  return {
    getSnapshot: () => snapshot ??= flattenSections(planSections(readPlanTree(doc))).map(({ key, title, number }) => ({ key, title, number })),
    subscribe: (notify: () => void) => { listeners.add(notify); return () => { listeners.delete(notify) } },
  }
}

export function sectionReferenceIndex(doc: Y.Doc) {
  let index = indexes.get(doc)
  if (!index) { index = createIndex(doc); indexes.set(doc, index) }
  return index
}

export function referenceLabel(doc: Y.Doc, id: string, untitled: string): string | null {
  const section = sectionReferenceIndex(doc).getSnapshot().find(section => section.key === id)
  return section ? `${section.number} ${section.title || untitled}` : null
}

export function searchReferenceSections(sections: readonly ReferenceSection[], query: string) {
  const words = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)
  return sections.filter(section => words.every(word => `${section.number} ${section.title}`.toLocaleLowerCase().includes(word)))
}
