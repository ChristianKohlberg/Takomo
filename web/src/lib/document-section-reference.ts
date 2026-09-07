import { Node, type Editor } from '@tiptap/react'
import * as Y from 'yjs'
import { nodesMap } from './mindmap-crdt'
import { specificationLink } from './specification-url'

export function sectionReferenceTitle(ydoc: Y.Doc, id: string): string | null {
  const entry = nodesMap(ydoc).get(id)
  if (!(entry instanceof Y.Map)) return null
  const value = entry.get('title')
  return value instanceof Y.Text ? value.toString() : typeof value === 'string' ? value : ''
}

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
      element.getAttribute('data-reference-project') === this.options.project() ? null : false }]
  },
  renderHTML({ node }) {
    const title = this.options.ydoc ? sectionReferenceTitle(this.options.ydoc, node.attrs.sectionId) : null
    return ['a', { 'data-section-id': node.attrs.sectionId, 'data-reference-project': this.options.project(),
      ...(title === null ? {} : { href: specificationLink(this.options.project(), 'document', node.attrs.sectionId) }) },
      (title ?? node.textContent) || this.options.untitledLabel()]
  },
  renderText({ node }) {
    return (this.options.ydoc && sectionReferenceTitle(this.options.ydoc, node.attrs.sectionId)) || node.textContent || this.options.untitledLabel()
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
        const title = options.ydoc ? sectionReferenceTitle(options.ydoc, current.attrs.sectionId) : null
        dom.dataset.sectionId = current.attrs.sectionId
        dom.dataset.referenceProject = options.project()
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
      const nodes = options.ydoc ? nodesMap(options.ydoc) : null
      const changed = (events: Y.YEvent<Y.AbstractType<unknown>>[]) => {
        if (events.some(event => event.path[0] === current.attrs.sectionId ||
          (event instanceof Y.YMapEvent && event.target === nodes && event.keysChanged.has(current.attrs.sectionId)))) refresh()
      }
      nodes?.observeDeep(changed)
      return {
        dom,
        update(next) { if (next.type !== current.type) return false; current = next; refresh(); return true },
        stopEvent: event => event.type === 'click' || event.type === 'mousedown',
        ignoreMutation: () => true,
        destroy: () => { storage.refresh.delete(refresh); nodes?.unobserveDeep(changed) },
      }
    }
  },
})
