import { Editor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { describe, expect, it, vi } from 'vitest'
import { DiagramCodeBlock } from './diagram-code-block'
import { BlockId } from './block-id'

const { mount } = vi.hoisted(() => ({ mount: vi.fn((_host: HTMLElement, _source: string, _engine: string, _access: unknown) => vi.fn()) }))
vi.mock('./diagram', async (original) => ({ ...await original<typeof import('./diagram')>(), mountDiagram: mount }))

describe('Mermaid code block view', () => {
  it('keeps source editable and block identity stable while refreshing the preview', () => {
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock, BlockId],
      content: { type: 'doc', content: [{ type: 'codeBlock', attrs: { language: 'mermaid', id: 'block-1' }, content: [{ type: 'text', text: 'flowchart TD\nA --> B' }] }] },
    })
    const view = editor.view.dom.querySelector('[data-id="block-1"]')!
    expect(view.querySelector('pre code')?.textContent).toBe('flowchart TD\nA --> B')
    expect(view.querySelector('pre')?.closest('[contenteditable="false"]')).toBeNull()
    expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'flowchart TD\nA --> B', 'mermaid', null)
    const cancel = mount.mock.results.at(-1)!.value
    editor.commands.insertContentAt(editor.state.doc.content.size - 1, '\nB --> C')
    expect(cancel).toHaveBeenCalled()
    expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'flowchart TD\nA --> B\nB --> C', 'mermaid', null)
    expect(editor.getJSON().content?.[0]?.attrs).toMatchObject({ id: 'block-1', language: 'mermaid' })
    expect(editor.getHTML()).not.toContain('Rendering diagram')
    const stop = mount.mock.results.at(-1)!.value
    editor.destroy()
    expect(stop).toHaveBeenCalled()
  })

  it('keeps view preferences out of the document and exposes source for a read-only reader', () => {
    localStorage.clear()
    const editor = new Editor({
      element: document.createElement('div'),
      editable: false,
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock],
      content: '<pre><code class="language-mermaid">flowchart TD\nA --> B</code></pre>',
    })
    const before = editor.getJSON()
    const updates = vi.fn()
    editor.on('update', updates)
    const root = editor.view.dom
    expect(root.querySelector('pre')?.hidden).toBe(true)
    const buttons = Array.from(root.querySelectorAll('button'))
    buttons.find(button => button.textContent === 'Code')!.click()
    expect(root.querySelector('pre')?.hidden).toBe(false)
    expect(editor.isEditable).toBe(false)
    buttons.find(button => button.textContent === 'View')!.click()
    expect(editor.getJSON()).toEqual(before)
    expect(updates).not.toHaveBeenCalled()
    editor.destroy()
    localStorage.clear()
  })

  it('opens empty Mermaid source for typing without saving an automatic preference', () => {
    localStorage.clear()
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock],
      content: { type: 'doc', content: [{ type: 'codeBlock', attrs: { language: 'mermaid' } }] },
    })
    expect(editor.view.dom.querySelector('pre')?.hidden).toBe(false)
    editor.commands.insertContentAt(1, 'flowchart TD\nA --> B')
    expect(editor.view.dom.querySelector('code')?.textContent).toBe('flowchart TD\nA --> B')
    expect(localStorage.getItem('takomo.mermaid.preferences.v1')).toBeNull()
    editor.destroy()
  })

  it.each(['plantuml', 'puml', 'salt', 'd2'])('renders %s for readers without changing source or granting editing', (language) => {
    localStorage.clear()
    const access = { token: 'reader', project: 'actual-project' }
    const editor = new Editor({
      element: document.createElement('div'), editable: false,
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock.configure({ access: () => access })],
      content: { type: 'doc', content: [{ type: 'codeBlock', attrs: { language }, content: [{ type: 'text', text: 'source' }] }] },
    })
    const before = editor.getJSON()
    expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'source', language === 'd2' ? 'd2' : 'plantuml', access)
    Array.from(editor.view.dom.querySelectorAll('button')).find(button => button.textContent === 'Code')!.click()
    expect(editor.isEditable).toBe(false)
    expect(editor.getJSON()).toEqual(before)
    editor.destroy()
    localStorage.clear()
  })

  it('discards old previews and credentials when access changes without a source edit', () => {
    localStorage.clear()
    const accessChanges = new EventTarget()
    let access = { token: 'first-token', project: 'first-project' }
    const editor = new Editor({
      element: document.createElement('div'), editable: false,
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock.configure({ access: () => access, accessChanges })],
      content: { type: 'doc', content: [{ type: 'codeBlock', attrs: { language: 'd2' }, content: [{ type: 'text', text: 'A -> B' }] }] },
    })
    const before = editor.getJSON()
    const updates = vi.fn()
    editor.on('update', updates)
    for (const next of [{ token: 'second-token', project: 'first-project' }, { token: 'second-token', project: 'second-project' }]) {
      const host = mount.mock.calls.at(-1)![0] as HTMLElement
      host.append(document.createElement('img'))
      const cancel = mount.mock.results.at(-1)!.value
      access = next
      accessChanges.dispatchEvent(new Event('change'))
      expect(cancel).toHaveBeenCalled()
      expect(editor.view.dom.querySelector('img')).toBeNull()
      expect(mount).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'A -> B', 'd2', next)
      expect(editor.getJSON()).toEqual(before)
      expect(updates).not.toHaveBeenCalled()
    }
    editor.destroy()
  })

  it('leaves ordinary code blocks as source without starting Mermaid', () => {
    mount.mockClear()
    const editor = new Editor({
      element: document.createElement('div'),
      extensions: [StarterKit.configure({ codeBlock: false }), DiagramCodeBlock],
      content: '<pre><code class="language-js">const value = 1</code></pre>',
    })
    expect(mount).not.toHaveBeenCalled()
    expect(editor.view.dom.querySelector('code')?.textContent).toBe('const value = 1')
    editor.destroy()
  })
})
