import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { describe, expect, it, vi } from 'vitest'
import { MermaidCodeBlock } from './mermaid-code-block'
import { BlockId } from './block-id'

const { mount } = vi.hoisted(() => ({ mount: vi.fn(() => vi.fn()) }))
vi.mock('./mermaid', () => ({ mountMermaid: mount }))

describe('Mermaid code block view', () => {
  it('keeps source editable and block identity stable while refreshing the preview', () => {
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: [StarterKit.configure({ codeBlock: false }), MermaidCodeBlock, BlockId],
      content: { type: 'doc', content: [{ type: 'codeBlock', attrs: { language: 'mermaid', id: 'block-1' }, content: [{ type: 'text', text: 'flowchart TD\nA --> B' }] }] },
    })
    const view = editor.view.dom.querySelector('[data-id="block-1"]')!
    expect(view.querySelector('pre code')?.textContent).toBe('flowchart TD\nA --> B')
    expect(view.querySelector('pre')?.closest('[contenteditable="false"]')).toBeNull()
    expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'flowchart TD\nA --> B')
    const cancel = mount.mock.results.at(-1)!.value
    editor.commands.insertContentAt(editor.state.doc.content.size - 1, '\nB --> C')
    expect(cancel).toHaveBeenCalled()
    expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'flowchart TD\nA --> B\nB --> C')
    expect(editor.getJSON().content?.[0]?.attrs).toMatchObject({ id: 'block-1', language: 'mermaid' })
    expect(editor.getHTML()).not.toContain('Rendering diagram')
    const stop = mount.mock.results.at(-1)!.value
    editor.destroy()
    expect(stop).toHaveBeenCalled()
  })

  it('leaves ordinary code blocks as source without starting Mermaid', () => {
    mount.mockClear()
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: [StarterKit.configure({ codeBlock: false }), MermaidCodeBlock],
      content: '<pre><code class="language-js">const value = 1</code></pre>',
    })
    expect(mount).not.toHaveBeenCalled()
    expect(editor.view.dom.querySelector('code')?.textContent).toBe('const value = 1')
    editor.destroy()
  })
})
