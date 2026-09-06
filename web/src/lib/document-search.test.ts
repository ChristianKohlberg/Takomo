import { afterEach, describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { getSchema } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import { prosemirrorToYXmlFragment } from 'y-prosemirror'
import { findDocumentMatches, fragmentMatches, literalMatches, proseMatches } from './document-search'

const schema = getSchema([StarterKit, TableKit])
function fixture() {
  const doc = new Y.Doc()
  const node = new Y.Map()
  doc.getMap('nodes').set('section', node)
  const fragment = new Y.XmlFragment()
  node.set('prose', fragment)
  return { doc, fragment }
}

afterEach(() => { vi.restoreAllMocks() })

describe('personal document search', () => {
  it('keys a prose result to its characters and reads each block once however many times the query hits', () => {
    const { doc, fragment } = fixture()
    prosemirrorToYXmlFragment(schema.nodeFromJSON({ type: 'doc', content: [
      { type: 'paragraph', content: [{ type: 'text', text: 'find ' }, { type: 'text', marks: [{ type: 'bold' }], text: 'find find find find find find find find find' }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'unique' }] },
    ] }), fragment)
    const nodes = [{ id: 'section', title: 'Section' }]
    const before = findDocumentMatches(nodes, doc, 'find')
    expect(before).toHaveLength(10)
    expect(new Set(before.map(m => m.key)).size).toBe(10)
    const [first, second] = before as [typeof before[number], typeof before[number]]
    ;(fragment.get(0) as Y.XmlElement).insert(0, [new Y.XmlText('Preface. ')])
    const after = findDocumentMatches(nodes, doc, 'find')
    expect(after[0]).toMatchObject({ key: first.key, from: first.from + 'Preface. '.length })
    expect(after[1]).toMatchObject({ key: second.key, from: second.from + 'Preface. '.length })
    ;((fragment.get(0) as Y.XmlElement).get(1) as Y.XmlText).delete(0, 'find '.length)
    expect(findDocumentMatches(nodes, doc, 'find').map(m => m.key)).not.toContain(first.key)
    expect(findDocumentMatches(nodes, doc, 'find').map(m => m.key)).toContain(second.key)
    const reads = vi.spyOn(Y.XmlElement.prototype, 'toArray')
    findDocumentMatches(nodes, doc, 'unique')
    const single = reads.mock.calls.length
    reads.mockClear()
    findDocumentMatches(nodes, doc, 'find')
    expect(reads.mock.calls.length).toBe(single)
  })
  it('treats punctuation literally, ignores case, and preserves Unicode offsets', () => {
    expect(literalMatches('A.b a.b axb İa.b', 'a.b')).toEqual([{ from: 0, to: 3 }, { from: 4, to: 7 }, { from: 13, to: 16 }])
    expect(literalMatches('anything', '')).toEqual([])
  })
  it('finds unmounted table, code, and formatted prose at editor positions', () => {
    const { doc, fragment } = fixture()
    const pm = schema.nodeFromJSON({ type: 'doc', content: [
      { type: 'paragraph', content: [{ type: 'text', text: 'Fi' }, { type: 'text', marks: [{ type: 'bold' }], text: 'nd' }, { type: 'hardBreak' }, { type: 'text', text: 'find' }] },
      { type: 'table', content: [{ type: 'tableRow', content: [{ type: 'tableCell', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Find in cell' }] }] }] }] },
      { type: 'codeBlock', content: [{ type: 'text', text: 'find()\nfind()' }] },
    ] })
    prosemirrorToYXmlFragment(pm, fragment)
    const before = Y.encodeStateAsUpdate(doc)
    expect(fragmentMatches(fragment, 'find')).toEqual(proseMatches(pm, 'find'))
    expect(fragmentMatches(fragment, 'find')).toHaveLength(5)
    const matches = findDocumentMatches([{ id: 'section', title: 'Find me' }], doc, 'find')
    expect(matches).toHaveLength(6)
    expect(matches[0]?.kind).toBe('heading')
    expect(Y.encodeStateAsUpdate(doc)).toEqual(before)
  })
  it('does not match across blocks or include deleted sections from a stale outline', () => {
    const { doc, fragment } = fixture()
    prosemirrorToYXmlFragment(schema.nodeFromJSON({ type: 'doc', content: [
      { type: 'paragraph', content: [{ type: 'text', text: 'find' }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'me' }] },
    ] }), fragment)
    expect(fragmentMatches(fragment, 'findme')).toEqual([])
    doc.getMap('nodes').delete('section')
    expect(findDocumentMatches([{ id: 'section', title: 'find' }], doc, 'find')).toEqual([])
  })
})
