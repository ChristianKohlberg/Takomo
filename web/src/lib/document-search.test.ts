import { describe, expect, it } from 'vitest'
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

describe('personal document search', () => {
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
