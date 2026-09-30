// The browser's accept, recorded as the contract the server's accept must meet.
//
// An agent may now accept a proposal over the API, and then Rust — not the
// editor — turns the proposal's markdown into CRDT nodes
// (`src/api/proposal_apply.rs`). A connected browser renders whatever lands, so
// a node with one attribute too few or a mark stored under the wrong key is a
// corrupted shared document that nobody sees until it misbehaves.
//
// So this test does not describe the Rust side at all. It runs the REAL accept —
// `applyOps` on a Tiptap editor with the SectionEditor schema, bound to a Y.Doc
// through Collaboration, block ids minted by the BlockId plugin — and records
// what reached the Y.XmlFragment: element names, every attribute, every text
// run and its formatting attributes, exactly as y-prosemirror wrote them. The
// Rust test `proposal_apply::tests::matches_the_browser_fixture` replays the
// same ops and must produce the same tree.
//
// The recording is committed at `tests/fixtures/proposal-markdown.json`. This
// test fails if the browser's output drifts from it; regenerate deliberately with
//   UPDATE_PROPOSAL_FIXTURES=1 npx vitest run src/lib/proposal-parity.test.ts
// and then make the Rust side agree.
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import * as Y from 'yjs'
import { Editor } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import { TableKit } from '@tiptap/extension-table'

import { BlockId } from './block-id'
import { CollapsibleBlock, CollapsibleContent, CollapsibleSummary } from './collapsible-block'
import { DiagramCodeBlock } from './diagram-code-block'
import { DocumentSectionReference } from './document-section-reference'
import { applyOps, type Op } from './doc-ops'
import { parseInline } from './inline-markdown'

const FIXTURE = resolve(import.meta.dirname, '../../../tests/fixtures/proposal-markdown.json')
const KNOWN = new Set(['blk_a', 'blk_b'])

/** Every case starts from two paragraphs, `blk_a` "Alpha" and `blk_b` "Beta". */
const CASES: { name: string; ops: Op[] }[] = [
  { name: 'plain paragraph', ops: [{ op: 'replace', id: 'blk_a', markdown: 'Just words.' }] },
  { name: 'every inline mark', ops: [{ op: 'replace', id: 'blk_a', markdown: 'Some **bold**, *italic*, _also italic_, __strong__, ~~gone~~, `code` and [a link](https://example.com/x?y=1).' }] },
  { name: 'nested emphasis', ops: [{ op: 'replace', id: 'blk_a', markdown: '**bold with *italic* inside** and ***both*** and **_mixed_**' }] },
  { name: 'escapes and strays stay literal', ops: [{ op: 'replace', id: 'blk_a', markdown: 'Literal \\*stars\\* and a stray * and snake_case_name and 2 * 3 * 4 and \\\\ and \\q' }] },
  { name: 'unmatched markers', ops: [{ op: 'replace', id: 'blk_a', markdown: '**open only, *half, ~~no end, `tick, [label](no close and [x] (y)' }] },
  { name: 'code spans are verbatim', ops: [{ op: 'replace', id: 'blk_a', markdown: 'Use `**not bold**` and ``a ` tick`` and `\\*raw\\*`' }] },
  { name: 'marks around code', ops: [{ op: 'replace', id: 'blk_a', markdown: '**bold `code` bold** and [`x`](https://e.com)' }] },
  { name: 'links', ops: [{ op: 'replace', id: 'blk_a', markdown: '[**b** c](/rel), [m](mailto:a@b.c), [bad](javascript:alert(1)), [ftp](ftp://x) and [a [nested] label](#frag)' }] },
  { name: 'heading with marks', ops: [{ op: 'replace', id: 'blk_a', markdown: '## A **bold** _heading_' }] },
  { name: 'deep heading and too deep', ops: [{ op: 'replace', id: 'blk_a', markdown: '###### Six\n\n####### Seven' }] },
  { name: 'bullet list with marks', ops: [{ op: 'replace', id: 'blk_a', markdown: '- **Milch** kaufen\n* Brot [hier](https://b.de)\n- `code` item' }] },
  { name: 'ordered list', ops: [{ op: 'replace', id: 'blk_a', markdown: '1. first *one*\n2. second\n10. tenth' }] },
  { name: 'blockquote', ops: [{ op: 'replace', id: 'blk_a', markdown: '> quoted **bold**\n>second line' }] },
  { name: 'code fences', ops: [{ op: 'replace', id: 'blk_a', markdown: '```rust\nfn main() { let x = **y**; }\n```\n\n```\nplain\n\nwith blank\n```' }] },
  { name: 'mermaid with blank line', ops: [{ op: 'insert_after', id: 'blk_a', markdown: 'Before.\n```mermaid\nflowchart TD\n\n  A --> B\n```\nAfter.' }] },
  { name: 'pipe table', ops: [{ op: 'replace', id: 'blk_a', markdown: '| Name | **Cost** | Note |\n| :--- | ---: | :---: |\n| Alice | 10 | `a\\|b` |\n| *Bob* | 20 | [x](https://x.de) |' }] },
  { name: 'not a table', ops: [{ op: 'replace', id: 'blk_a', markdown: '| a | b |\n| -- | -- |' }] },
  { name: 'horizontal rule', ops: [{ op: 'insert_after', id: 'blk_a', markdown: '---' }] },
  { name: 'multi-block insert', ops: [{ op: 'insert_after', id: 'blk_a', markdown: 'One **para**.\n\n- a\n- b\n\n### Head\n\nLast line\nwrapped here.' }] },
  { name: 'replace then insert on one anchor', ops: [
    { op: 'replace', id: 'blk_a', markdown: 'New A.\n\nAnd its tail.' },
    { op: 'insert_after', id: 'blk_a', markdown: 'After the tail.' },
    { op: 'insert_after', id: 'blk_a', markdown: 'After that.' },
  ] },
  { name: 'delete and a missing block', ops: [
    { op: 'delete', id: 'blk_b' },
    { op: 'replace', id: 'blk_gone', markdown: 'x' },
    { op: 'insert_after', id: 'blk_b', markdown: 'y' },
  ] },
  { name: 'unicode', ops: [{ op: 'replace', id: 'blk_a', markdown: '**Größe** _über_ 😀 *ß* und ~~Maß~~ — `ü`' }] },
  { name: 'whitespace rules', ops: [{ op: 'replace', id: 'blk_a', markdown: '** not bold** and *not italic * and a*b*c and x_y_z and _ok_.' }] },
]

/**
 * Inline strings for a direct grammar cross-check: hand-picked edge cases, then
 * a seeded pseudo-random soup of every marker the grammar knows. The random
 * half is what finds the off-by-ones a hand-picked list never thinks of.
 */
function inlineCases(): string[] {
  const picked = [
    '', 'plain', '**a**', '__a__', '*a*', '_a_', '~~a~~', '`a`', '``a`b``', '[a](b)',
    '***a***', '**a*b*c**', '*a **b** c*', '_a __b__ c_', '**a** **b**', 'a**b**c', 'a__b__c',
    'x_y_z', '_x_y', '*a', 'a*', '* a*', '*a *', '**a', '~a~', '~~ a~~', '`', '``', '` `',
    '[a]', '[a](', '[a]()', '[a](b c)', '[a](javascript:x)', '[[a]](b)', '[a](<b>)', '\\*a\\*',
    '\\`a`', '\\[a](b)', '[`a`](b)', '[*a*](http://b)', '**[a](b)**', 'ä_ö_ü', '_ä_', '1_2_3',
    '__init__', '**bold `code** x`', 'a ~~b ~~ c~~', '***', '****', '*****a*****', '_*a*_', '*_a_*',
  ]
  const alphabet = ['*', '*', '_', '_', '~', '`', '[', ']', '(', ')', '\\', 'a', 'b', ' ', ' ', 'ü', '1', 'http://x', 'ftp:y', '.']
  let seed = 20260930
  const next = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648)
  const random: string[] = []
  for (let n = 0; n < 400; n++) {
    const len = 1 + Math.floor(next() * 14)
    let out = ''
    for (let i = 0; i < len; i++) out += alphabet[Math.floor(next() * alphabet.length)]
    random.push(out)
  }
  return [...picked, ...random]
}

function canonical(node: Y.XmlElement | Y.XmlText | Y.XmlHook, top: boolean): unknown {
  if (node instanceof Y.XmlText) {
    return {
      text: node.toDelta().map((d: { insert: string; attributes?: Record<string, unknown> }) =>
        d.attributes && Object.keys(d.attributes).length ? { insert: d.insert, attributes: d.attributes } : { insert: d.insert }),
    }
  }
  if (!(node instanceof Y.XmlElement)) throw new Error('unexpected Y node')
  const attrs: Record<string, unknown> = { ...node.getAttributes() }
  if (top && typeof attrs.id === 'string' && !KNOWN.has(attrs.id)) {
    expect(attrs.id).toMatch(/^blk_[0-9a-z]+$/)
    attrs.id = '<fresh>'
  }
  return {
    type: node.nodeName,
    attrs,
    content: node.toArray().map((c) => canonical(c as Y.XmlElement | Y.XmlText, false)),
  }
}

let editor: Editor | undefined
afterEach(() => editor?.destroy())

function run(ops: Op[]) {
  const doc = new Y.Doc()
  const frag = doc.getXmlFragment('prose')
  for (const [id, text] of [['blk_a', 'Alpha'], ['blk_b', 'Beta']] as const) {
    const p = new Y.XmlElement('paragraph')
    p.setAttribute('id', id)
    p.insert(0, [new Y.XmlText(text)])
    frag.push([p])
  }
  editor = new Editor({
    extensions: [
      StarterKit.configure({ undoRedo: false, codeBlock: false }),
      CollapsibleBlock, CollapsibleSummary, CollapsibleContent,
      DiagramCodeBlock,
      TableKit.configure({ table: { resizable: true } }),
      Collaboration.configure({ document: doc, fragment: frag }),
      BlockId.configure({ canWrite: true }),
      DocumentSectionReference,
    ],
  })
  const tr = editor.state.tr
  const { applied, skipped } = applyOps(tr, editor.schema, ops)
  editor.view.dispatch(tr)
  const result = {
    applied,
    skipped,
    fragment: frag.toArray().map((n) => canonical(n as Y.XmlElement, true)),
  }
  editor.destroy()
  editor = undefined
  return result
}

describe('proposal parity fixture', () => {
  it('records what the browser writes into the CRDT for each proposal', () => {
    const cases = CASES.map((c) => ({ name: c.name, ops: c.ops, ...run(c.ops) }))
    const inline = inlineCases().map((text) => ({ text, runs: parseInline(text) }))
    const fixture = { note: 'Generated by web/src/lib/proposal-parity.test.ts. Asserted by src/api/proposal_apply.rs. Do not edit by hand.', start: [['blk_a', 'Alpha'], ['blk_b', 'Beta']], cases, inline }
    if (process.env.UPDATE_PROPOSAL_FIXTURES) writeFileSync(FIXTURE, JSON.stringify(fixture, null, 1) + '\n')
    expect(JSON.parse(readFileSync(FIXTURE, 'utf8'))).toEqual(JSON.parse(JSON.stringify(fixture)))
  })
})
