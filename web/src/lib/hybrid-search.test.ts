import { describe, expect, it } from 'vitest'
import { Schema, type Node } from '@tiptap/pm/model'
import { excerptRanges, passageRange, type SearchResult } from './hybrid-search'
const result: SearchResult = { node_id: 'n', title: '', heading_path: [], excerpt: 'İstanbul and <script>abc</script>', passage: '', highlights: ['<script>', 'script', 'İstanbul'], match_kind: 'keyword' }
const schema = new Schema({ nodes: { doc: { content: 'paragraph+' }, paragraph: { content: 'inline*', group: 'block' }, text: { group: 'inline' }, hard_break: { inline: true, group: 'inline' } }, marks: { strong: {} } })
const paragraph = (...content: Node[]) => schema.node('paragraph', null, content)
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
  it('maps a passage across the blank and whitespace-only paragraphs the index skipped', () => {
    const doc = schema.node('doc', null, [
      paragraph(schema.text('First point')),
      paragraph(),
      paragraph(schema.text('   ')),
      paragraph(schema.text('Second point')),
    ])
    // The server stores "First point\nSecond point" for this section.
    const range = passageRange(doc, 'First point\nSecond point')
    expect(range).not.toBeNull()
    expect(doc.textBetween(range!.from, range!.to, '\n')).toBe('First point\n\n   \nSecond point')
    expect(doc.resolve(range!.from).parent.textContent).toBe('First point')
    expect(doc.resolve(range!.to).parent.textContent).toBe('Second point')
  })
  it('treats a hard break as a line boundary the way the index does', () => {
    const doc = schema.node('doc', null, [paragraph(schema.text('before'), schema.node('hard_break'), schema.text('after'))])
    const range = passageRange(doc, 'before\nafter')
    expect(range).not.toBeNull()
    expect(doc.textBetween(range!.from, range!.to, '', '\n')).toBe('before\nafter')
  })
})
