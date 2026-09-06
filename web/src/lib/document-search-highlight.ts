import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view'
import { proseMatches } from './document-search'

export interface DocumentSearchHighlightState { query: string; activeFrom?: number }
const key = new PluginKey<DocumentSearchHighlightState>('documentSearch')
export function setDocumentSearchHighlight(view: EditorView, search: DocumentSearchHighlightState): void {
  view.dispatch(view.state.tr.setMeta(key, search))
}
export const DocumentSearchHighlight = Extension.create({
  name: 'documentSearchHighlight',
  addProseMirrorPlugins() {
    return [new Plugin<DocumentSearchHighlightState>({
      key,
      state: {
        init: () => ({ query: '' }),
        apply: (transaction, state) => (transaction.getMeta(key) as DocumentSearchHighlightState | undefined) ?? state,
      },
      props: {
        decorations(state) {
          const search = key.getState(state)
          if (!search?.query) return DecorationSet.empty
          return DecorationSet.create(state.doc, proseMatches(state.doc, search.query).map(match => Decoration.inline(match.from, match.to, {
            class: match.from === search.activeFrom ? 'document-search-match document-search-active' : 'document-search-match',
            'data-document-search-active': match.from === search.activeFrom ? 'true' : 'false',
          })))
        },
      },
    })]
  },
})
