import { descendantsOf, type MapNode } from './mindmap-doc'
import { layoutForMode, type CanvasMode } from './mindmap-custom-layout'
import { NODE_HEIGHT, NODE_WIDTH, type Point } from './mindmap-layout'

/** A local projection: never reparent the shared document to focus a branch. */
export function focusBranch(nodes: MapNode[], root: string | null): MapNode[] {
  if (!root) return nodes
  const ids = new Set([root, ...descendantsOf(nodes, root)])
  return nodes.filter(node => ids.has(node.id)).map(node => node.id === root ? { ...node, parent: null } : node)
}

export function focusedLayout(nodes: MapNode[], mode: CanvasMode, customRoot?: Point, focusRoot?: string | null) {
  const placed = layoutForMode(nodes, mode, customRoot)
  if (focusRoot && placed.nodes.length) placed.bounds = {
    minX: Math.min(...placed.nodes.map(node => node.x)),
    minY: Math.min(...placed.nodes.map(node => node.y)),
    maxX: Math.max(...placed.nodes.map(node => node.x)) + NODE_WIDTH,
    maxY: Math.max(...placed.nodes.map(node => node.y)) + NODE_HEIGHT,
  }
  return placed
}
