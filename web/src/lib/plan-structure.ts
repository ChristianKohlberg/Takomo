import * as Y from 'yjs'
import { between } from './fracdex'
import { nodesMap, readPlanTree } from './mindmap-crdt'
import type { PlanNode } from './plan-sections'

export type SectionPlacement = 'before' | 'after' | 'child'
export type StructureResult = { ok: true } | { ok: false; error: 'missing' | 'cycle' | 'changed' | 'empty' }
const validParent = (tree: PlanNode[], id: string, parent: string | null): boolean => {
  const seen = new Set([id])
  while (parent !== null) {
    if (seen.has(parent)) return false
    seen.add(parent)
    const node = tree.find(n => n.id === parent)
    if (!node) return false
    parent = node.parent
  }
  return true
}

const stamp = (doc: Y.Doc, id: string): void => {
  doc.transact(() => { nodesMap(doc).get(id)?.set('updated_at', Date.now()) })
}

/** Only parent/order change: prose, children, links and identity stay in place. */
export function movePlanSection(doc: Y.Doc, id: string, target: string, placement: SectionPlacement, origin?: unknown): StructureResult {
  const tree = readPlanTree(doc)
  const source = tree.find(n => n.id === id)
  const destination = tree.find(n => n.id === target)
  if (!source || !destination) return { ok: false, error: 'missing' }
  const parent = placement === 'child' ? target : destination.parent
  if (id === target || !validParent(tree, id, parent)) return { ok: false, error: 'cycle' }
  const siblings = tree.filter(n => n.parent === parent && n.id !== id)
  const index = placement === 'child' ? siblings.length : siblings.findIndex(n => n.id === target) + (placement === 'after' ? 1 : 0)
  const order = between(siblings[index - 1]?.order ?? null, siblings[index]?.order ?? null)
  doc.transact(() => {
    const node = nodesMap(doc).get(id)!
    node.set('parent', parent)
    node.set('order', order)
    node.set('x', null)
    node.set('y', null)
  }, origin)
  stamp(doc, id)
  return { ok: true }
}

type Position = { parent: string | null; order: string }
type Move = { id: string; before: Position; after: Position }

/** Session-local history. A unique origin excludes both remote writes and prose.
 * Validate the live hierarchy before Yjs restores individual fields; never
 * replace the document with a snapshot or resurrect a deleted destination. */
export function createStructureHistory(doc: Y.Doc) {
  const origin = {}
  const manager = new Y.UndoManager(nodesMap(doc), { trackedOrigins: new Set([origin]), captureTimeout: 0 })
  const listeners = new Set<() => void>()
  const notify = () => listeners.forEach(listener => listener())
  const key = Symbol('move')
  manager.on('stack-item-added', notify)
  manager.on('stack-item-popped', notify)
  const run = (direction: 'undo' | 'redo'): StructureResult => {
    const stack = direction === 'undo' ? manager.undoStack : manager.redoStack
    const item = stack.at(-1)
    if (!item) return { ok: false, error: 'empty' }
    const move = item.meta.get(key) as Move | undefined
    if (!move) return { ok: false, error: 'changed' }
    const tree = readPlanTree(doc)
    const current = tree.find(n => n.id === move.id)
    if (!current) return { ok: false, error: 'missing' }
    const expected = direction === 'undo' ? move.after : move.before
    const restore = direction === 'undo' ? move.before : move.after
    if (current.parent !== expected.parent || current.order !== expected.order) return { ok: false, error: 'changed' }
    if (restore.parent !== null && !tree.some(n => n.id === restore.parent)) return { ok: false, error: 'missing' }
    if (!validParent(tree, move.id, restore.parent)) return { ok: false, error: 'cycle' }
    manager[direction]()
    stamp(doc, move.id)
    const opposite = direction === 'undo' ? manager.redoStack : manager.undoStack
    opposite.at(-1)?.meta.set(key, move)
    notify()
    return { ok: true }
  }
  return {
    get undoSection(): string | undefined { return (manager.undoStack.at(-1)?.meta.get(key) as Move | undefined)?.id },
    get redoSection(): string | undefined { return (manager.redoStack.at(-1)?.meta.get(key) as Move | undefined)?.id },
    get canUndo() { return manager.undoStack.length > 0 },
    get canRedo() { return manager.redoStack.length > 0 },
    move(id: string, target: string, placement: SectionPlacement): StructureResult {
      const before = readPlanTree(doc).find(n => n.id === id)
      const result = movePlanSection(doc, id, target, placement, origin)
      if (result.ok && before) {
        const after = readPlanTree(doc).find(n => n.id === id)!
        manager.undoStack.at(-1)?.meta.set(key, { id, before: { parent: before.parent, order: before.order }, after: { parent: after.parent, order: after.order } } satisfies Move)
        notify()
      }
      return result
    },
    undo: () => run('undo'),
    redo: () => run('redo'),
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener) } },
    destroy() { manager.destroy(); listeners.clear() },
  }
}
