// Marking the blocks a pending proposal is about.
//
// Decorations, not marks — and that distinction is the whole reason this file is
// three dozen lines instead of a feature.
//
// A mark would be *content*: written into the shared document, synced to every
// peer, merged, undoable, and needing to be cleaned up again when the proposal
// is decided. Every one of those is a way for a highlight to outlive the thing
// it was highlighting.
//
// A decoration is a **view artifact**. It is computed locally from state each
// peer already has, it never touches the CRDT, and it disappears when the
// proposal does because there is nothing to remove. The document is unchanged
// while a proposal is pending, which is exactly what "nothing an agent writes is
// live text" is supposed to mean — a highlight that had to be written into the
// document would quietly break that promise.
import { Extension } from '@tiptap/react'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import type { EditorView } from '@tiptap/pm/view'
import type { Node as PMNode } from '@tiptap/pm/model'

export const highlightKey = new PluginKey<DecorationSet>('highlightBlocks')

/** Ids currently under a pending proposal, sent in as a transaction meta. */
interface HighlightMeta {
  ids: string[]
}

export function setHighlightedBlocks(view: EditorView, ids: Set<string>): void {
  const meta: HighlightMeta = { ids: [...ids] }
  view.dispatch(view.state.tr.setMeta(highlightKey, meta))
}

function decorate(doc: PMNode, ids: Set<string>): DecorationSet {
  if (!ids.size) return DecorationSet.empty
  const found: Decoration[] = []
  doc.forEach((node, offset) => {
    const id = node.attrs.id as string | undefined
    if (id && ids.has(id)) {
      found.push(
        Decoration.node(offset, offset + node.nodeSize, {
          class: 'takomo-proposed',
          // Announced rather than only coloured: somebody reading with a screen
          // reader has the same right to know a change is being offered here.
          'aria-describedby': 'takomo-proposal-note',
        }),
      )
    }
  })
  return DecorationSet.create(doc, found)
}

export const HighlightBlocks = Extension.create({
  name: 'highlightBlocks',

  addProseMirrorPlugins() {
    return [
      new Plugin<DecorationSet>({
        key: highlightKey,
        state: {
          init: () => DecorationSet.empty,
          apply(tr, old) {
            const meta = tr.getMeta(highlightKey) as HighlightMeta | undefined
            if (meta) return decorate(tr.doc, new Set(meta.ids))
            // No new instruction: carry the existing set through this change.
            // `map` is what keeps a highlight on the right block while somebody
            // types above it.
            return old.map(tr.mapping, tr.doc)
          },
        },
        props: {
          decorations: (state) => highlightKey.getState(state),
        },
      }),
    ]
  },
})
