import { diagramEngine, type DiagramAccess } from './diagram'
import CodeBlock from '@tiptap/extension-code-block'
import { createDiagramControls } from './diagram-controls'

/** Keep the existing codeBlock schema/CRDT source; preview is local view only. */
export const DiagramCodeBlock = CodeBlock.extend<{ access: () => DiagramAccess | null; accessChanges: EventTarget | null }>({
  addOptions() { return { ...this.parent?.(), access: () => null, accessChanges: null } },
  addNodeView() {
    return ({ node, editor, getPos }) => {
      const dom = document.createElement('div')
      const pre = document.createElement('pre')
      const code = document.createElement('code')
      pre.append(code)
      dom.append(pre)
      const accessChanges = this.options.accessChanges
      let current = node
      let currentAccess = this.options.access()
      let controls: ReturnType<typeof createDiagramControls> | undefined
      const refresh = () => {
        const engine = diagramEngine(current.attrs.language)
        const nextAccess = this.options.access()
        const accessChanged = nextAccess?.token !== currentAccess?.token || nextAccess?.project !== currentAccess?.project
        currentAccess = nextAccess
        if (current.attrs.id) dom.setAttribute('data-id', String(current.attrs.id))
        else dom.removeAttribute('data-id')
        if (current.attrs.language) code.className = `language-${current.attrs.language}`
        else code.removeAttribute('class')
        if (engine) {
          if (controls && (dom.dataset.diagramEngine !== engine || accessChanged)) { controls.destroy(); controls = undefined }
          dom.dataset.diagramEngine = engine
          if (controls) controls.update(current.textContent)
          else controls = createDiagramControls(dom, pre, current.textContent, editor.isEditable && !current.textContent, engine, currentAccess)
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
      const refreshAccess = () => {
        const next = this.options.access()
        if (next?.token !== currentAccess?.token || next?.project !== currentAccess?.project) refresh()
      }
      accessChanges?.addEventListener('change', refreshAccess)
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
          accessChanges?.removeEventListener('change', refreshAccess)
          editor.off('selectionUpdate', revealSelection)
          controls?.destroy()
        },
      }
    }
  },
})
