import CodeBlock from '@tiptap/extension-code-block'
import { mountMermaid } from './mermaid'

/** Keep the existing codeBlock schema/CRDT source; preview is local view only. */
export const MermaidCodeBlock = CodeBlock.extend({
  addNodeView() {
    return ({ node }) => {
      const dom = document.createElement('div')
      const preview = document.createElement('div')
      preview.contentEditable = 'false'
      const pre = document.createElement('pre')
      const code = document.createElement('code')
      pre.append(code)
      dom.append(preview, pre)
      let current = node
      let cancel: (() => void) | undefined
      const refresh = () => {
        cancel?.()
        const mermaid = String(current.attrs.language ?? '').toLowerCase() === 'mermaid'
        preview.hidden = !mermaid
        preview.replaceChildren()
        if (current.attrs.id) dom.setAttribute('data-id', String(current.attrs.id))
        else dom.removeAttribute('data-id')
        if (current.attrs.language) code.className = `language-${current.attrs.language}`
        else code.removeAttribute('class')
        if (mermaid) cancel = mountMermaid(preview, current.textContent)
      }
      refresh()
      return {
        dom,
        contentDOM: code,
        update(next) {
          if (next.type !== current.type) return false
          const changed = next.textContent !== current.textContent || next.attrs.language !== current.attrs.language || next.attrs.id !== current.attrs.id
          current = next
          if (changed) refresh()
          return true
        },
        ignoreMutation(mutation) {
          return mutation.type !== 'selection' && (mutation.target === preview || preview.contains(mutation.target))
        },
        destroy() { cancel?.() },
      }
    }
  },
})
