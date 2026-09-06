// Browser-produced Yjs bytes exercise the actual cross-language document reader.
// Regenerate from web/: node scripts/document-table-fixture.mjs
import { getSchema } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import { prosemirrorJSONToYDoc } from 'y-prosemirror'
import * as Y from 'yjs'
import { URL } from 'node:url'
import { writeFileSync } from 'node:fs'
const p = text => ({ type: 'paragraph', content: [{ type: 'text', text, marks: [{ type: 'bold' }] }] })
const json = { type: 'doc', content: [{ type: 'table', content: [
  { type: 'tableRow', content: [{ type: 'tableHeader', attrs: { colspan: 2, colwidth: [140, 180] }, content: [p('Plan & cost')] }] },
  { type: 'tableRow', content: [
    { type: 'tableCell', attrs: { rowspan: 2, colwidth: [140] }, content: [p('Owner'), { type: 'bulletList', content: [{ type: 'listItem', content: [p('Alice')] }] }] },
    { type: 'tableCell', attrs: { colwidth: [180] }, content: [p('First')] },
  ] },
  { type: 'tableRow', content: [{ type: 'tableCell', attrs: { colwidth: [180] }, content: [p('Second')] }] },
] }] }
const schema = getSchema([StarterKit, TableKit])
const doc = prosemirrorJSONToYDoc(schema, json, 'prose')
doc.getXmlFragment('prose').get(0).setAttribute('id', 'blk_table')
writeFileSync(new URL('../../tests/fixtures/document-table.yjs', import.meta.url), Y.encodeStateAsUpdate(doc))
doc.destroy()
