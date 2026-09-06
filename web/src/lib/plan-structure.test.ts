import { describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { createNode, nodesMap, readPlanTree, setTitle } from './mindmap-crdt'
import { createStructureHistory, movePlanSection } from './plan-structure'

function setup() {
  const doc = new Y.Doc()
  const a = createNode(doc, { title: 'A', parent: null, by: 'test' })!
  const b = createNode(doc, { title: 'B', parent: null, by: 'test' })!
  const child = createNode(doc, { title: 'Child', parent: a, by: 'test' })!
  return { doc, a, b, child }
}

describe('document structure moves and local history', () => {
  it('moves the existing subtree and shared fields, preserving identities', () => {
    const { doc, a, b, child } = setup()
    const node = nodesMap(doc).get(a)!
    const prose = node.get('prose')
    node.set('test_link', 'test-123')
    expect(movePlanSection(doc, a, b, 'after')).toEqual({ ok: true })
    expect(readPlanTree(doc).filter(n => n.parent === null).map(n => n.id)).toEqual([b, a])
    expect(nodesMap(doc).get(a)).toBe(node)
    expect(node.get('prose')).toBe(prose)
    expect(node.get('test_link')).toBe('test-123')
    expect(nodesMap(doc).get(child)?.get('parent')).toBe(a)
    expect(movePlanSection(doc, a, b, 'child')).toEqual({ ok: true })
    expect(node.get('parent')).toBe(b)
  })
  it('refuses cycles and missing targets without changing content', () => {
    const { doc, a, child } = setup()
    const before = Y.encodeStateAsUpdate(doc)
    expect(movePlanSection(doc, a, child, 'child')).toEqual({ ok: false, error: 'cycle' })
    expect(movePlanSection(doc, a, 'missing', 'after')).toEqual({ ok: false, error: 'missing' })
    expect(Y.encodeStateAsUpdate(doc)).toEqual(before)
  })
  it('undoes only this session moves and retains remote text edits', () => {
    const { doc, a, b } = setup()
    const history = createStructureHistory(doc)
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(doc))
    history.move(a, b, 'child')
    setTitle(peer, a, 'Remote title')
    const paragraph = new Y.XmlElement('paragraph')
    const text = new Y.XmlText()
    paragraph.insert(0, [text])
    text.insert(0, 'Remote prose')
    ;(nodesMap(peer).get(a)!.get('prose') as Y.XmlFragment).insert(0, [paragraph])
    Y.applyUpdate(doc, Y.encodeStateAsUpdate(peer))
    expect(history.undo()).toEqual({ ok: true })
    expect(readPlanTree(doc).find(n => n.id === a)).toMatchObject({ parent: null, title: 'Remote title' })
    expect((nodesMap(doc).get(a)!.get('prose') as Y.XmlFragment).toString()).toContain('Remote prose')
    expect(history.redoSection).toBe(a)
    expect(history.redo()).toEqual({ ok: true })
    expect(readPlanTree(doc).find(n => n.id === a)?.parent).toBe(b)
    history.destroy()
  })
  it('refuses undo when the old parent vanished, and redo when a collaborator moved the node', () => {
    const { doc, a, b, child } = setup()
    const history = createStructureHistory(doc)
    history.move(child, b, 'child')
    nodesMap(doc).delete(a)
    expect(history.undo()).toEqual({ ok: false, error: 'missing' })
    const other = setup()
    const h = createStructureHistory(other.doc)
    h.move(other.a, other.b, 'child')
    expect(h.undo()).toEqual({ ok: true })
    const peer = new Y.Doc()
    Y.applyUpdate(peer, Y.encodeStateAsUpdate(other.doc))
    movePlanSection(peer, other.a, other.b, 'after')
    Y.applyUpdate(other.doc, Y.encodeStateAsUpdate(peer))
    expect(h.redo()).toEqual({ ok: false, error: 'changed' })
  })
})
