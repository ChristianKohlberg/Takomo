import * as Y from 'yjs'
import { nodesMap, readNodes } from './mindmap-crdt'
import {
  COLUMN_GAP, NODE_HEIGHT, NODE_WIDTH, ROW_GAP, layout, radialLayout,
  type Layout, type LayoutNode, type Point,
} from './mindmap-layout'

export type CanvasMode = 'custom' | 'radial' | 'tidy'
const CUSTOM_LAYOUT_SYNC = 'mindmap-custom-layout-sync'

export function readCustomRoot(doc: Y.Doc): Point | undefined {
  const root = doc.getMap('layout').get('customRoot') as Point | undefined
  return root && Number.isFinite(root.x) && Number.isFinite(root.y) ? root : undefined
}

/** Automatic views never consume or overwrite the saved hand placement. */
export function layoutForMode<T extends LayoutNode>(
  nodes: readonly T[], mode: CanvasMode, customRoot?: Point,
): Layout<T> {
  if (mode !== 'custom') {
    const automatic = nodes.map(node => ({ ...node, at: null }))
    return mode === 'radial' ? radialLayout(automatic) : layout(automatic)
  }
  const placed = radialLayout(nodes)
  if (!customRoot) return placed
  placed.root = customRoot
  // New thoughts grow beside their saved parent. Only vacant positions are
  // assigned; adding or deleting a thought never reflows the saved arrangement.
  const byId = new Map(placed.nodes.map(p => [p.node.id, p]))
  const occupied: Point[] = [customRoot, ...placed.nodes.filter(p => p.node.at).map(p => p.node.at!)]
  for (const p of [...placed.nodes].sort((a, b) => a.depth - b.depth)) {
    if (p.node.at) continue
    const parent = p.node.parent ? byId.get(p.node.parent) : customRoot
    if (parent) {
      const direction = parent.x >= customRoot.x ? 1 : -1
      p.x = parent.x + direction * (NODE_WIDTH + COLUMN_GAP)
      p.y = parent.y
    }
    while (occupied.some(at =>
      Math.abs(at.x - p.x) < NODE_WIDTH + COLUMN_GAP / 2 &&
      Math.abs(at.y - p.y) < NODE_HEIGHT + ROW_GAP,
    )) p.y += NODE_HEIGHT + ROW_GAP
    occupied.push({ x: p.x, y: p.y })
  }
  const points = [customRoot, ...placed.nodes]
  placed.bounds = {
    minX: Math.min(...points.map(p => p.x)),
    minY: Math.min(...points.map(p => p.y)),
    maxX: Math.max(...points.map(p => p.x)) + NODE_WIDTH,
    maxY: Math.max(...points.map(p => p.y)) + NODE_HEIGHT,
  }
  return placed
}

/** Called only after sync by a writer. Existing x/y remain the one shared,
 * durable custom arrangement; the root records its initial coordinate frame.
 * This bookkeeping has its own origin so it cannot become a user's undo step. */
export function ensureCustomLayout(doc: Y.Doc, initialMode: 'radial' | 'tidy'): void {
  const nodes = readNodes(doc)
  if (!nodes.length) return
  const root = readCustomRoot(doc)
  if (root && nodes.every(node => node.at)) return
  const placed = root
    ? layoutForMode(nodes, 'custom', root)
    : initialMode === 'tidy' ? layout(nodes) : radialLayout(nodes)
  doc.transact(() => {
    if (!root) doc.getMap('layout').set('customRoot', placed.root)
    for (const p of placed.nodes) {
      const node = nodesMap(doc).get(p.node.id)
      if (!node || (typeof node.get('x') === 'number' && typeof node.get('y') === 'number')) continue
      node.set('x', p.x)
      node.set('y', p.y)
    }
  }, CUSTOM_LAYOUT_SYNC)
}
