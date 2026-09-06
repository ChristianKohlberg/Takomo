import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import type * as Y from 'yjs'
import { COMMENT_FIELD, readCommentThreads, resolveCommentAnchor } from './document-comments'

const key = new PluginKey('documentComments')
export const DocumentCommentHighlight = Extension.create<{ ydoc: Y.Doc | null; sectionId: string; onOpen?: () => void }>({
  name: 'documentCommentHighlight',
  addOptions: () => ({ ydoc: null, sectionId: '' }),
  addProseMirrorPlugins() {
    const { ydoc, sectionId, onOpen } = this.options
    const editor = this.editor
    if (!ydoc || !sectionId) return []
    return [new Plugin({
      key,
      view(view) {
        const comments = ydoc.getMap(COMMENT_FIELD)
        const refresh = () => { if (!view.isDestroyed) view.dispatch(view.state.tr.setMeta(key, true)) }
        comments.observeDeep(refresh)
        return { destroy: () => comments.unobserveDeep(refresh) }
      },
      props: {
        handleClick(view, _pos, event) {
          if (!view.state.selection.empty || !(event.target instanceof Element) || !event.target.closest('[data-comment-id]')) return false
          onOpen?.()
          return false
        },
        decorations(state) {
          const marks = readCommentThreads(ydoc, sectionId).filter(thread => !thread.resolved).flatMap(thread => {
            const range = resolveCommentAnchor(editor, thread.anchor)
            return range ? [Decoration.inline(range.from, range.to, { class: 'document-comment-anchor', 'data-comment-id': thread.id })] : []
          })
          return DecorationSet.create(state.doc, marks)
        },
      },
    })]
  },
})
