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

const passageKey = new PluginKey<{ from: number; to: number } | null>('searchPassage')
/** Local decoration only: never enters document content or Yjs. */
export function highlightSearchPassage(view: EditorView, range: { from: number; to: number }): void {
  view.dispatch(view.state.tr.setMeta(passageKey, range))
}
export const SearchPassageHighlight = Extension.create({
  name: 'searchPassageHighlight',
  addProseMirrorPlugins() {
    return [new Plugin<{ from: number; to: number } | null>({
      key: passageKey,
      state: {
        init: () => null,
        apply: (transaction, state) => transaction.docChanged ? null : transaction.getMeta(passageKey) !== undefined ? transaction.getMeta(passageKey) as typeof state : state,
      },
      view(view) {
        let timer: ReturnType<typeof setTimeout> | undefined
        let previous = passageKey.getState(view.state)
        return {
          update() {
            const next = passageKey.getState(view.state)
            if (next !== previous) {
              previous = next
              clearTimeout(timer)
              if (next) timer = setTimeout(() => {
                if (!view.isDestroyed) view.dispatch(view.state.tr.setMeta(passageKey, null))
              }, 3000)
            }
          },
          destroy() { clearTimeout(timer) },
        }
      },
      props: {
        decorations(state) {
          const range = passageKey.getState(state)
          return range ? DecorationSet.create(state.doc, [Decoration.inline(range.from, range.to, { class: 'document-search-match document-search-active', 'data-search-passage': 'true' })]) : DecorationSet.empty
        },
      },
    })]
  },
})
