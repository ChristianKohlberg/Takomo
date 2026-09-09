import { Node, type Editor } from '@tiptap/react'
import { Fragment } from '@tiptap/pm/model'
import * as Y from 'yjs'
import { referenceLabel, sectionReferenceIndex } from './section-reference-index'
import { specificationLink } from './specification-url'

export { sectionReferenceTitle } from './mindmap-crdt'

/** Labels are read at render time, so a locale change redraws mounted references in place. */
export function refreshSectionReferenceLabels(editor: Editor): void {
  const storage = (editor.storage as unknown as Record<string, { refresh?: Set<() => void> } | undefined>).sectionReference
  storage?.refresh?.forEach(refresh => refresh())
}

/** Identity is shared content; the current title is presentation, never a rename-triggered prose edit. */
export const DocumentSectionReference = Node.create<{
  onNavigate: ((id: string) => void) | null; ydoc: Y.Doc | null; project: () => string; missingLabel: () => string; untitledLabel: () => string
}, { refresh: Set<() => void> }>({
  name: 'sectionReference', priority: 1100,
  group: 'inline', inline: true, atom: true, selectable: true, content: 'text*',
  addOptions: () => ({ onNavigate: null, ydoc: null, project: () => '', missingLabel: () => 'Missing section', untitledLabel: () => 'Untitled section' }),
  addStorage: () => ({ refresh: new Set<() => void>() }),
  addAttributes: () => ({
    sectionId: { default: '', parseHTML: element => element.getAttribute('data-section-id') },
  }),
  parseHTML() {
    return [{ tag: 'a[data-section-id]', priority: 100, getAttrs: element =>
      element.getAttribute('data-reference-project') === this.options.project() ? null : false,
      getContent: (element, schema) => {
        // Display labels include current titles and localized missing markers.
        // Clipboard round trips must retain the raw shared fallback instead.
        const fallback = (element as HTMLElement).getAttribute('data-reference-fallback') ?? element.textContent ?? ''
        return fallback ? Fragment.from(schema.text(fallback)) : Fragment.empty
      },
    }]
  },
  renderHTML({ node }) {
    const title = this.options.ydoc ? referenceLabel(this.options.ydoc, node.attrs.sectionId, this.options.untitledLabel()) : null
    return ['a', { class: 'document-section-reference', 'data-section-id': node.attrs.sectionId, 'data-reference-project': this.options.project(),
      'data-reference-fallback': node.textContent,
      ...(title === null ? { 'aria-disabled': 'true' } : { href: specificationLink(this.options.project(), 'document', node.attrs.sectionId) }) },
      title === null ? `${node.textContent || this.options.untitledLabel()} (${this.options.missingLabel()})` : title || this.options.untitledLabel()]
  },
  renderText({ node }) {
    const title = this.options.ydoc ? referenceLabel(this.options.ydoc, node.attrs.sectionId, this.options.untitledLabel()) : null
    return title === null ? `${node.textContent || this.options.untitledLabel()} (${this.options.missingLabel()})` : title || this.options.untitledLabel()
  },
  addNodeView() {
    const options = this.options
    const storage = this.storage
    return ({ node }) => {
      let current = node
      const dom = document.createElement('a')
      dom.contentEditable = 'false'
      dom.className = 'document-section-reference'
      const refresh = () => {
        const title = options.ydoc ? referenceLabel(options.ydoc, current.attrs.sectionId, options.untitledLabel()) : null
        dom.dataset.sectionId = current.attrs.sectionId
        dom.dataset.referenceProject = options.project()
        dom.dataset.referenceFallback = current.textContent
        dom.textContent = title === null
          ? `${current.textContent || options.untitledLabel()} (${options.missingLabel()})`
          : title || options.untitledLabel()
        if (title === null) {
          dom.removeAttribute('href')
          dom.setAttribute('aria-disabled', 'true')
        } else {
          dom.href = specificationLink(options.project(), 'document', current.attrs.sectionId)
          dom.removeAttribute('aria-disabled')
        }
      }
      dom.addEventListener('click', event => {
        if (event.button !== 0 || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey ||
            !dom.hasAttribute('href') || !options.onNavigate) return
        event.preventDefault()
        options.onNavigate(current.attrs.sectionId)
      })
      refresh()
      storage.refresh.add(refresh)
      const unsubscribe = options.ydoc ? sectionReferenceIndex(options.ydoc).subscribe(refresh) : null
      return {
        dom,
        update(next) { if (next.type !== current.type) return false; current = next; refresh(); return true },
        stopEvent: event => event.type === 'click' || event.type === 'mousedown',
        ignoreMutation: () => true,
        destroy: () => { storage.refresh.delete(refresh); unsubscribe?.() },
      }
    }
  },
})
