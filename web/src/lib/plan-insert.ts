import type * as Y from 'yjs'
import { between, isValid } from './fracdex'
import { createNode, nodesMap, readPlanTree } from './mindmap-crdt'
import { flattenSections, planSections } from './plan-sections'

/** Insert at a document boundary using the same tree the canvas renders.
 * A heading nests under the nearest preceding heading one level above it.
 * Skipped levels are refused instead of silently creating a different level. */
export function insertPlanSection(
  doc: Y.Doc,
  after: string | null,
  level: 1 | 2 | 3,
  title: string,
  by: string,
): string | null {
  const heading = title.trim()
  if (!heading) return null
  const tree = readPlanTree(doc)
  const rows = flattenSections(planSections(tree))
  const index = after === null ? -1 : rows.findIndex((row) => row.key === after)
  if (after !== null && index < 0) return null
  if (level > (rows[index]?.depth ?? -1) + 2) return null
  const preceding = rows.slice(0, index + 1).reverse()
  const parent = level === 1 ? null : preceding.find((row) => row.depth === level - 2)?.key
  if (parent === undefined) return null
  const siblings = tree.filter((node) => node.parent === parent)
  const previous = preceding.find((row) => siblings.some((node) => node.id === row.key))
  let id: string | null = null
  doc.transact(() => {
    id = createNode(doc, { parent, after: previous?.key, title: heading, by })
    // createNode's null anchor appends. An insertion before any sibling needs
    // the key before the first sibling instead, in this same transaction.
    if (id && !previous && siblings.length) {
      const first = siblings[0]!.order
      nodesMap(doc).get(id)?.set('order', between(null, isValid(first) ? first : null))
    }
  })
  return id
}
