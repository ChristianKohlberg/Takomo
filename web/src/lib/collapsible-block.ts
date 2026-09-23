import { Node, mergeAttributes, type Editor } from '@tiptap/react'
import { Selection, TextSelection } from '@tiptap/pm/state'

interface Labels { title: string; unwrap: string; expand: string; collapse: string }
declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    collapsibleBlock: { wrapCollapsibleBlock: (title?: string) => ReturnType; unwrapCollapsibleBlock: () => ReturnType }
  }
}

export function refreshCollapsibleLabels(editor: Editor): void {
  const storage = (editor.storage as unknown as Record<string, { refresh?: Set<() => void> } | undefined>).collapsibleBlock
  storage?.refresh?.forEach(refresh => refresh())
}

/** Content is shared; expanding a block is a local reading preference, never a CRDT edit. */
export const CollapsibleBlock = Node.create<{ labels: () => Labels }, { refresh: Set<() => void> }>({
  name: 'collapsibleBlock', group: 'block', content: 'collapsibleSummary collapsibleContent', defining: true,
  addStorage: () => ({ refresh: new Set() }),
  addOptions: () => ({ labels: () => ({ title: 'Details', unwrap: 'Remove folding', expand: 'Expand', collapse: 'Collapse' }) }),
  parseHTML: () => [{ tag: 'details[data-collapsible-block]' }],
  renderHTML: ({ HTMLAttributes }) => ['details', mergeAttributes(HTMLAttributes, { 'data-collapsible-block': '' }), 0],
  addCommands() {
    return {
      wrapCollapsibleBlock: title => ({ state, tr, dispatch, editor }) => {
        if (!editor.isEditable || editor.isActive(this.name)) return false
        const { $from, $to } = tr.selection
        // Wrap complete top-level blocks, including a table when the caret is in a cell.
        const from = $from.depth ? $from.before(1) : $from.pos
        const to = $to.depth ? $to.after(1) : $to.pos
        const content = tr.doc.slice(from, to).content
        if (!content.size) return false
        const summary = state.schema.nodes.collapsibleSummary!.create(null, state.schema.text(title?.trim() || this.options.labels().title))
        const body = state.schema.nodes.collapsibleContent!.create(null, content)
        if (dispatch) {
          tr.replaceWith(from, to, this.type.create(null, [summary, body]))
          tr.setSelection(TextSelection.create(tr.doc, from + 2))
        }
        return true
      },
      unwrapCollapsibleBlock: () => ({ tr, dispatch, editor }) => {
        if (!editor.isEditable) return false
        const { $from } = tr.selection
        for (let depth = $from.depth; depth > 0; depth--) {
          const block = $from.node(depth)
          if (block.type !== this.type) continue
          if (dispatch) tr.replaceWith($from.before(depth), $from.after(depth), block.child(1).content)
          return true
        }
        return false
      },
    }
  },
  addKeyboardShortcuts() {
    return {
      Enter: () => {
        const { $from, empty } = this.editor.state.selection
        if (!this.editor.isEditable || !empty || $from.parent.type.name !== 'collapsibleSummary') return false
        // Enter from the editable summary moves into the body and reveals it.
        return this.editor.commands.command(({ tr, dispatch }) => {
          if (dispatch) tr.setSelection(Selection.near(tr.doc.resolve($from.after() + 1))).scrollIntoView()
          return true
        })
      },
    }
  },
  addNodeView() {
    return ({ editor, getPos, node }) => {
      let current = node
      const dom = document.createElement('div')
      dom.className = 'document-collapsible'
      dom.dataset.collapsibleBlock = ''
      const content = document.createElement('div')
      content.className = 'document-collapsible-prose'
      content.id = `collapsible-${crypto.randomUUID()}`
      const toggle = document.createElement('button')
      toggle.type = 'button'
      toggle.contentEditable = 'false'
      toggle.className = 'document-collapsible-toggle'
      toggle.setAttribute('aria-controls', content.id)
      let open = false
      const controls = document.createElement('div')
      controls.contentEditable = 'false'
      controls.className = 'document-collapsible-controls'
      const unwrap = document.createElement('button')
      unwrap.type = 'button'
      unwrap.textContent = this.options.labels().unwrap
      controls.append(unwrap)
      dom.append(toggle, content, controls)
      const refresh = () => {
        controls.hidden = !open || !editor.isEditable
        dom.dataset.expanded = String(open)
        toggle.setAttribute('aria-expanded', String(open))
        const labels = this.options.labels()
        toggle.setAttribute('aria-label', `${open ? labels.collapse : labels.expand}: ${current.firstChild?.textContent || labels.title}`)
        const icon = open ? '▾' : '▸'
        if (toggle.textContent !== icon) toggle.textContent = icon
        const label = this.options.labels().unwrap
        if (unwrap.textContent !== label) unwrap.textContent = label
      }
      this.storage.refresh.add(refresh)
      toggle.addEventListener('click', () => { open = !open; refresh() })
      dom.addEventListener('reveal-collapsible', event => { if (event.target === dom) { open = true; refresh() } })
      unwrap.addEventListener('click', () => {
        const pos = getPos()
        if (!editor.isEditable || pos == null) return
        // Unwrap the latest mapped block, preserving remote edits and its child IDs.
        editor.chain().focus().setTextSelection(pos + 2).unwrapCollapsibleBlock().run()
      })
      const revealSelection = () => {
        const pos = getPos()
        const { from, to } = editor.state.selection
        if (pos != null && to > pos + 1 + current.firstChild!.nodeSize && from < pos + current.nodeSize - 1) {
          open = true
          refresh()
        }
      }
      editor.on('selectionUpdate', revealSelection)
      refresh()
      return {
        dom, contentDOM: content,
        update(updated) { if (updated.type !== current.type) return false; current = updated; refresh(); return true },
        stopEvent: event => controls.contains(event.target as globalThis.Node) || toggle.contains(event.target as globalThis.Node),
        ignoreMutation: mutation => mutation.type !== 'selection' && (mutation.target === dom && mutation.type === 'attributes' || controls.contains(mutation.target) || toggle.contains(mutation.target)),
        destroy: () => { editor.off('selectionUpdate', revealSelection); this.storage.refresh.delete(refresh) },
      }
    }
  },
})
export const CollapsibleSummary = Node.create({
  name: 'collapsibleSummary', content: 'inline*', defining: true,
  parseHTML: () => [{ tag: 'summary' }], renderHTML: () => ['summary', 0],
  // Native editable <summary> can retain an old range when it toggles. Keep
  // title editing and the disclosure button separate in the live editor.
  addNodeView: () => ({ editor, getPos }) => {
    const dom = document.createElement('div')
    dom.className = 'document-collapsible-summary'
    dom.addEventListener('mousedown', event => {
      if (event.button !== 0 || event.shiftKey || !editor.isEditable) return
      const pos = getPos()
      if (pos == null) return
      const summary = editor.state.doc.nodeAt(pos)
      if (!summary) return
      const hit = editor.view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos ?? pos + 1
      // Browser selectionchange is deferred. Immediately discard an old search
      // range, even when the click lands in the title's empty padding.
      editor.commands.setTextSelection(Math.max(pos + 1, Math.min(pos + summary.nodeSize - 1, hit)))
    })
    return { dom, contentDOM: dom }
  },
})
export const CollapsibleContent = Node.create({
  name: 'collapsibleContent', content: 'block+', defining: true,
  parseHTML: () => [{ tag: 'div[data-collapsible-content]' }],
  renderHTML: () => ['div', { 'data-collapsible-content': '' }, 0],
})
export const CollapsibleExtensions = [CollapsibleBlock, CollapsibleSummary, CollapsibleContent]
