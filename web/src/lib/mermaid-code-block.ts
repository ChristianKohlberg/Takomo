import CodeBlock from '@tiptap/extension-code-block'
import { createMermaidControls } from './mermaid-controls'

/** Keep the existing codeBlock schema/CRDT source; preview is local view only. */
export const MermaidCodeBlock = CodeBlock.extend({
  addNodeView() {
    return ({ node, editor, getPos }) => {
      const dom = document.createElement('div')
      const pre = document.createElement('pre')
      const code = document.createElement('code')
      pre.append(code)
      dom.append(pre)
      let current = node
      let controls: ReturnType<typeof createMermaidControls> | undefined
      const refresh = () => {
        const mermaid = String(current.attrs.language ?? '').toLowerCase() === 'mermaid'
        if (current.attrs.id) dom.setAttribute('data-id', String(current.attrs.id))
        else dom.removeAttribute('data-id')
        if (current.attrs.language) code.className = `language-${current.attrs.language}`
        else code.removeAttribute('class')
        if (mermaid) {
          if (controls) controls.update(current.textContent)
          else controls = createMermaidControls(dom, pre, current.textContent, editor.isEditable && !current.textContent)
        } else {
          controls?.destroy()
          controls = undefined
        }
      }
      // Arrow navigation / insertion into a diagram must reveal the actual
      // contentDOM before the writer continues typing into their selection.
      const revealSelection = () => {
        if (!editor.isEditable || !editor.isFocused || !controls) return
        const pos = getPos()
        const { from, to } = editor.state.selection
        if (typeof pos === 'number' && from > pos && to < pos + current.nodeSize) controls.showCode()
      }
      editor.on('selectionUpdate', revealSelection)
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
        stopEvent(event) {
          return event.target instanceof Node && !!controls?.controls.contains(event.target)
        },
        ignoreMutation(mutation) {
          if (mutation.type === 'selection') return false
          return mutation.target === dom || (mutation.type === 'attributes' && mutation.target === pre)
            || !!controls?.controls.contains(mutation.target)
        },
        destroy() {
          editor.off('selectionUpdate', revealSelection)
          controls?.destroy()
        },
      }
    }
  },
})
