import { describe, expect, it } from 'vitest'
import { getSchema } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'

import { markdownToNodes } from './doc-ops'
import { parseInline, safeHref } from './inline-markdown'

const schema = getSchema([StarterKit, TableKit])

/** A node's text runs as `[text, mark names]`, for compact assertions. */
function runsOf(markdown: string): [string, string[]][] {
  const out: [string, string[]][] = []
  markdownToNodes(schema, markdown)[0]!.descendants((n) => {
    if (n.isText) out.push([n.text!, n.marks.map((m) => (m.type.name === 'link' ? `link:${m.attrs.href}` : m.type.name))])
  })
  return out
}

describe('inline markdown in proposals', () => {
  it('turns the delimiters into the editor’s marks instead of literal characters', () => {
    expect(runsOf('a **b** *c* _d_ ~~e~~ `f` [g](https://h.io)')).toEqual([
      ['a ', []], ['b', ['bold']], [' ', []], ['c', ['italic']], [' ', []], ['d', ['italic']],
      [' ', []], ['e', ['strike']], [' ', []], ['f', ['code']], [' ', []], ['g', ['link:https://h.io']],
    ])
  })

  it('applies marks in headings, list items, quotes and pipe-table cells', () => {
    expect(runsOf('## A **b**')).toEqual([['A ', []], ['b', ['bold']]])
    expect(runsOf('- *x*\n- y')).toEqual([['x', ['italic']], ['y', []]])
    expect(runsOf('> `q`')).toEqual([['q', ['code']]])
    expect(runsOf('| **H** |\n| --- |\n| _c_ |')).toEqual([['H', ['bold']], ['c', ['italic']]])
  })

  it('never parses inside code, keeps escapes and unmatched markers literal', () => {
    expect(runsOf('`**x**` \\*y\\* 2 * 3 snake_case_name *open')).toEqual([
      ['**x**', ['code']], [' *y* 2 * 3 snake_case_name *open', []],
    ])
  })

  it('lets code exclude every other mark, as the schema does', () => {
    expect(runsOf('**a `b` c** [`d`](https://e.io)')).toEqual([
      ['a ', ['bold']], ['b', ['code']], [' c', ['bold']], [' ', []], ['d', ['code']],
    ])
  })

  it('only links to http(s), mailto and relative destinations', () => {
    expect(safeHref('https://x')).toBe(true)
    expect(safeHref('/a/b')).toBe(true)
    expect(safeHref('#frag')).toBe(true)
    expect(safeHref('mailto:a@b')).toBe(true)
    expect(safeHref('javascript:alert(1)')).toBe(false)
    expect(runsOf('[x](javascript:alert(1))')).toEqual([['[x](javascript:alert(1))', []]])
  })

  it('stays fast on pathological input', () => {
    const started = performance.now()
    expect(parseInline('*a '.repeat(10_000))).toHaveLength(1)
    expect(performance.now() - started).toBeLessThan(2000)
  })
})
