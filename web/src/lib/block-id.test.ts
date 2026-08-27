import { describe, expect, it } from 'vitest'
import { Schema } from '@tiptap/pm/model'

import { annotatedMarkdown, blockId } from './block-id'

// A minimal schema rather than Tiptap's: this function is pure over a
// ProseMirror doc, so it is testable without booting an editor — which is the
// reason it is a plain function and not a method on one.
const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    text: { group: 'inline' },
    paragraph: {
      group: 'block',
      content: 'inline*',
      attrs: { id: { default: null } },
    },
    heading: {
      group: 'block',
      content: 'inline*',
      attrs: { id: { default: null }, level: { default: 1 } },
    },
    codeBlock: {
      group: 'block',
      content: 'text*',
      attrs: { id: { default: null }, language: { default: '' } },
    },
  },
})

const para = (id: string | null, text: string) =>
  schema.nodes.paragraph!.create({ id }, text ? schema.text(text) : null)

describe('blockId', () => {
  it('is prefixed so an id is recognizable wherever it turns up', () => {
    // The prefix is what lets an agent op, a commitment and a check all name the
    // same thing without a type tag beside it.
    expect(blockId()).toMatch(/^blk_[a-z0-9]+$/)
  })

  it('does not repeat itself', () => {
    const ids = new Set(Array.from({ length: 200 }, () => blockId()))
    expect(ids.size).toBe(200)
  })
})

describe('annotatedMarkdown', () => {
  it('puts each block id in a comment above its block, which is what the agent reads', () => {
    const doc = schema.nodes.doc!.create(null, [
      schema.nodes.heading!.create({ id: 'blk_aaa', level: 2 }, schema.text('Pricing')),
      para('blk_bbb', 'Our current tiers are…'),
    ])
    expect(annotatedMarkdown(doc)).toBe(
      '<!-- blk_aaa -->\n## Pricing\n\n<!-- blk_bbb -->\nOur current tiers are…',
    )
  })

  it('renders a heading at its own level rather than flattening it', () => {
    const doc = schema.nodes.doc!.create(null, [
      schema.nodes.heading!.create({ id: 'blk_a', level: 3 }, schema.text('Deep')),
    ])
    expect(annotatedMarkdown(doc)).toBe('<!-- blk_a -->\n### Deep')
  })

  it('fences a code block so its contents cannot be read as prose', () => {
    const doc = schema.nodes.doc!.create(null, [
      schema.nodes.codeBlock!.create({ id: 'blk_c', language: 'sh' }, schema.text('ls -la')),
    ])
    expect(annotatedMarkdown(doc)).toBe('<!-- blk_c -->\n```sh\nls -la\n```')
  })

  it('omits the comment for a block that has no id yet', () => {
    // A block can legitimately be idless for one transaction — the plugin fills
    // it in on the next. Emitting `<!-- null -->` would give the agent an id to
    // address that resolves to nothing.
    const doc = schema.nodes.doc!.create(null, [para(null, 'Fresh')])
    expect(annotatedMarkdown(doc)).toBe('Fresh')
  })
})
