import { describe, expect, it } from 'vitest'
import { Schema } from '@tiptap/pm/model'
import { excerptRanges, passageRange, type SearchResult } from './hybrid-search'
const result: SearchResult = { node_id: 'n', title: '', heading_path: [], excerpt: 'İstanbul and <script>abc</script>', passage: '', highlights: ['<script>', 'script', 'İstanbul'], match_kind: 'keyword' }
const schema = new Schema({ nodes: { doc: { content: 'paragraph+' }, paragraph: { content: 'text*', group: 'block' }, text: { group: 'inline' } }, marks: { strong: {} } })
describe('hybrid search evidence', () => {
  it('merges actual literal spans while preserving Unicode offsets', () => {
    expect(excerptRanges(result).map(range => result.excerpt.slice(range.from, range.to))).toEqual(['İstanbul', '<script>', 'script'])
    expect(excerptRanges({ ...result, match_kind: 'semantic' })).toEqual([])
  })
  it('maps an exact passage across formatting and paragraph boundaries', () => {
    const doc = schema.node('doc', null, [schema.node('paragraph', null, [schema.text('First '), schema.text('bold', [schema.mark('strong')])]), schema.node('paragraph', null, schema.text('Second paragraph'))])
    const range = passageRange(doc, 'bold\nSecond')
    expect(range).not.toBeNull()
    expect(doc.textBetween(range!.from, range!.to, '\n')).toBe('bold\nSecond')
    expect(passageRange(doc, 'changed source')).toBeNull()
  })
})
