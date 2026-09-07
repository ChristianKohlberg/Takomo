import { Extension } from '@tiptap/react'
import { AllSelection, NodeSelection, Plugin, PluginKey, TextSelection } from '@tiptap/pm/state'
import { getRelativeSelection, relativePositionToAbsolutePosition, ySyncPluginKey, yUndoPluginKey } from '@tiptap/y-tiptap'

/** y-tiptap saves the historical selection after the undo transaction has
 * already rendered. Apply its relative anchors now, against the restored doc,
 * instead of letting its absolute offsets leak into the next transaction's
 * structural recovery (where they can be beyond the current document's end).
 * Live remote selection recovery is unchanged. */
type HistoryEvent = { stackItem: { meta: Map<unknown, unknown> } }

const historySelectionKey = new PluginKey<ReturnType<typeof getRelativeSelection> | null>('collaborationHistorySelection')

export const CollaborationHistorySelection = Extension.create({
  name: 'collaborationHistorySelection',
  // Install after Collaboration's stack-item-popped listener.
  priority: 50,
  addProseMirrorPlugins() {
    return [new Plugin({
      key: historySelectionKey,
      state: {
        init: () => null,
        apply(transaction, previous, oldState) {
          if (transaction.getMeta('appendedTransaction')) return previous
          const binding = ySyncPluginKey.getState(oldState)?.binding
          if (!binding) return null
          // During a Yjs-originated change its mapping already describes the
          // new document. The binding captured the old selection before that
          // rebuild; recomputing against oldState here gives wrong anchors.
          return transaction.getMeta(ySyncPluginKey)?.isChangeOrigin
            ? binding.beforeTransactionSelection
            : getRelativeSelection(binding, oldState)
        },
      },
      view(view) {
        const manager = yUndoPluginKey.getState(view.state)?.undoManager
        const binding = ySyncPluginKey.getState(view.state)?.binding
        if (!manager || !binding) return {}
        const restore = ({ stackItem }: HistoryEvent) => {
          const saved = stackItem.meta.get(binding) as ReturnType<typeof getRelativeSelection> | null | undefined
          binding.beforeTransactionSelection = null
          if (!saved) return
          const doc = view.state.doc
          const resolve = (position: typeof saved.anchor) => position === null ? null :
            relativePositionToAbsolutePosition(binding.doc, binding.type, position, binding.mapping)
          const anchor = resolve(saved.anchor)
          const head = resolve(saved.head)
          let selection
          if (saved.type === 'all') selection = new AllSelection(doc)
          else if (saved.type === 'node' && anchor !== null && anchor <= doc.content.size && doc.nodeAt(anchor) && NodeSelection.isSelectable(doc.nodeAt(anchor)!)) {
            selection = NodeSelection.create(doc, anchor)
          } else if (anchor !== null || head !== null) {
            const from = Math.max(0, Math.min(anchor ?? head!, doc.content.size))
            const to = Math.max(0, Math.min(head ?? anchor!, doc.content.size))
            selection = TextSelection.between(doc.resolve(from), doc.resolve(to))
          }
          if (selection && !selection.eq(view.state.selection)) view.dispatch(view.state.tr.setSelection(selection))
        }
        const remember = ({ stackItem }: HistoryEvent) => {
          stackItem.meta.set(binding, historySelectionKey.getState(view.state))
        }
        manager.on('stack-item-added', remember)
        manager.on('stack-item-popped', restore)
        return { destroy: () => { manager.off('stack-item-added', remember); manager.off('stack-item-popped', restore) } }
      },
    })]
  },
})
