import { afterEach, describe, expect, it } from 'vitest'
import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { TableKit } from '@tiptap/extension-table'
import { CleanPaste, cleanPastedHTML } from './clean-paste'

let editor: Editor | undefined
afterEach(() => editor?.destroy())
function paste(html: string) {
  editor = new Editor({ extensions: [StarterKit, TableKit, CleanPaste], content: '<p></p>' })
  editor.view.pasteHTML(html, new Event('paste') as ClipboardEvent)
  return editor
}

describe('clean paste', () => {
  it('uses project typography while retaining Google Docs emphasis and links', () => {
    const result = paste(`<b id="docs-internal-guid" style="font-weight:normal"><p style="font-family:Arial;font-size:28px;color:red;background:yellow"><span style="font-weight:700">Bold</span> <span style="font-style:italic">italic</span> <a href="https://example.com">link</a></p></b>`)
    expect(result.getText()).toBe('Bold italic link')
    expect(result.getHTML()).toContain('<strong>Bold</strong>')
    expect(result.getHTML()).toContain('<em>italic</em>')
    expect(result.getHTML()).toContain('href="https://example.com"')
    expect(result.getHTML()).not.toMatch(/Arial|28px|red|yellow|docs-internal-guid/)
    expect(result.state.doc.firstChild!.child(1).marks).toEqual([])
  })

  it('keeps an explicit nested reset inside a styled or semantic emphasis container', () => {
    const result = paste('<p style="font-weight:700">Bold <span style="font-weight:400">normal</span></p>' +
      '<p style="font-style:italic">Italic <span style="font-style:normal">upright <span style="font-style:italic">again</span></span></p>' +
      '<p><b>Strong <span style="font-weight:normal">plain</span></b></p>' +
      '<p><u style="text-decoration:underline">Under <span style="text-decoration:none">none</span></u></p>')
    const marks = (block: number, index: number) => result.state.doc.child(block).child(index).marks.map(mark => mark.type.name)
    const texts = (block: number) => { const out: string[] = []; result.state.doc.child(block).forEach(node => out.push(node.text ?? '')); return out }
    expect(texts(0)).toEqual(['Bold ', 'normal'])
    expect(marks(0, 0)).toEqual(['bold'])
    expect(marks(0, 1)).toEqual([])
    expect(texts(1)).toEqual(['Italic ', 'upright ', 'again'])
    expect(marks(1, 0)).toEqual(['italic'])
    expect(marks(1, 1)).toEqual([])
    expect(marks(1, 2)).toEqual(['italic'])
    expect(texts(2)).toEqual(['Strong ', 'plain'])
    expect(marks(2, 0)).toEqual(['bold'])
    expect(marks(2, 1)).toEqual([])
    expect(texts(3)).toEqual(['Under ', 'none'])
    expect(marks(3, 0)).toEqual(['underline'])
    expect(marks(3, 1)).toEqual([])
    expect(result.getHTML()).not.toMatch(/style=/)
  })

  it('keeps headings, nested lists, quotes and tables editable', () => {
    const result = paste('<h2 style="font-size:50px">Heading</h2><ol start="3"><li>First<ul><li>Nested</li></ul></li></ol><blockquote>Quote</blockquote><table style="width:900px;font-weight:bold"><tbody><tr><th colspan="2">Header</th></tr><tr><td>A</td><td>B</td></tr></tbody></table>')
    const doc = result.state.doc
    const types: string[] = []
    doc.forEach(node => types.push(node.type.name))
    expect(types).toEqual(['heading', 'orderedList', 'blockquote', 'table', 'paragraph'])
    expect(doc.child(0).attrs.level).toBe(2)
    expect(doc.child(1).attrs.start).toBe(3)
    expect(doc.child(1).child(0).child(1).type.name).toBe('bulletList')
    expect(doc.child(3).child(0).child(0).attrs.colspan).toBe(2)
    expect(result.getText()).toContain('Nested')
  })

  it('keeps code language and whitespace but removes copied block identities and sizing', () => {
    const html = cleanPastedHTML('<pre data-id="blk_old"><code class="language-mermaid styled" style="font-size:30px">graph TD\n  A --&gt; B</code></pre><table><tr><td data-colwidth="700" colspan="2">cell</td></tr></table>')
    expect(html).toContain('class="language-mermaid"')
    expect(html).toContain('graph TD\n  A --&gt; B')
    expect(html).toContain('colspan="2"')
    expect(html).not.toMatch(/blk_old|data-colwidth|700|30px|styled/)
  })

  it('retains internal slice metadata and semantic custom-node data', () => {
    const html = cleanPastedHTML('<p data-pm-slice="1 1 []"><a href="/projects/test/specification?view=document&amp;section=mn-123" data-section-id="mn-123">Section</a></p>')
    expect(html).toContain('data-pm-slice="1 1 []"')
    expect(html).toContain('data-section-id="mn-123"')
  })

  it('supports undoing a paste', () => {
    const result = paste('<p><strong>New prose</strong></p>')
    expect(result.getText()).toBe('New prose')
    result.commands.undo()
    expect(result.getText()).toBe('')
  })
})
