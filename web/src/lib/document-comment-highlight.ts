import { Extension, type Editor } from '@tiptap/react'
import { Plugin, PluginKey, type EditorState, type Transaction } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import type * as Y from 'yjs'
import { ySyncPluginKey } from '@tiptap/y-tiptap'
import { COMMENT_FIELD, readCommentThreads, resolveCommentAnchor } from './document-comments'

const key = new PluginKey<DecorationSet>('documentComments')

function build(editor: Editor, ydoc: Y.Doc, sectionId: string, doc: ProseMirrorNode): DecorationSet {
  const marks = readCommentThreads(ydoc, sectionId).filter(thread => !thread.resolved).flatMap(thread => {
    const range = resolveCommentAnchor(editor, thread.anchor, doc)
    return range ? [Decoration.inline(range.from, range.to, { class: 'document-comment-anchor', 'data-comment-id': thread.id }, { id: thread.id })] : []
  })
  return DecorationSet.create(doc, marks)
}

function signature(set: DecorationSet): string {
  return set.find().map(d => `${d.from}:${d.to}:${String(d.spec.id)}`).sort().join('|')
}

export const DocumentCommentHighlight = Extension.create<{ ydoc: Y.Doc | null; sectionId: string; onOpen?: () => void }>({
  name: 'documentCommentHighlight',
  addOptions: () => ({ ydoc: null, sectionId: '' }),
  addProseMirrorPlugins() {
    const { ydoc, sectionId, onOpen } = this.options
    const editor = this.editor
    if (!ydoc || !sectionId) return []
    const rebuild = (doc: ProseMirrorNode) => build(editor, ydoc, sectionId, doc)
    return [new Plugin<DecorationSet>({
      key,
      state: {
        init: () => DecorationSet.empty,
        apply(tr: Transaction, set: DecorationSet, _old: EditorState, state: EditorState) {
          if (tr.getMeta(key) || tr.getMeta(ySyncPluginKey)) return rebuild(state.doc)
          return tr.docChanged ? set.map(tr.mapping, tr.doc) : set
        },
      },
      view(view) {
        const comments = ydoc.getMap(COMMENT_FIELD)
        const refresh = () => { if (!view.isDestroyed) view.dispatch(view.state.tr.setMeta(key, true)) }
        comments.observeDeep(refresh)
        let reconciling = false
        return {
          update(current, previous) {
            if (current.state.doc === previous.doc || reconciling) return
            reconciling = true
            queueMicrotask(() => {
              reconciling = false
              if (current.isDestroyed) return
              const mapped = key.getState(current.state)
              if (mapped && signature(mapped) !== signature(rebuild(current.state.doc))) refresh()
            })
          },
          destroy: () => comments.unobserveDeep(refresh),
        }
      },
      props: {
        handleClick(view, _pos, event) {
          if (!view.state.selection.empty || !(event.target instanceof Element) || !event.target.closest('[data-comment-id]')) return false
          onOpen?.()
          return false
        },
        decorations: state => key.getState(state) ?? null,
      },
    })]
  },
})
