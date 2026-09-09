import { Extension, type Editor } from '@tiptap/react'
import { closeHistory } from '@tiptap/pm/history'
import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state'
import type * as Y from 'yjs'
import { sectionReferenceIndex } from './section-reference-index'
export interface ReferenceMatch { from: number; to: number; query: string }
const key = new PluginKey<number | null>('sectionReferenceTrigger')
function matchAt(state: Pick<EditorState, 'doc' | 'selection' | 'storedMarks'>, from: number | null | undefined): ReferenceMatch | null {
  const { $from, empty, to } = state.selection
  if (from == null || !empty || from < $from.start() || from >= to || to - from > 100 || $from.parent.type.spec.code ||
    (state.storedMarks ?? $from.marks()).some(mark => ['code', 'link'].includes(mark.type.name))) return null
  const text = state.doc.textBetween(from, to, '\n', '\ufffc')
  return text.startsWith('@') && !/[\n@\ufffc]/.test(text.slice(1)) ? { from, to, query: text.slice(1) } : null
}
export function referenceMatch(state: EditorState) { return matchAt(state, key.getState(state)) }
export function closeReferenceMenu(editor: Editor) { editor.view.dispatch(editor.state.tr.setMeta(key, { close: true })) }
export const SectionReferenceTrigger = Extension.create<{
  menuId: string; onMatch: (match: ReferenceMatch | null) => void; onKey: (event: KeyboardEvent) => boolean
}>({
  name: 'sectionReferenceTrigger',
  addOptions: () => ({ menuId: 'section-reference-picker', onMatch: () => {}, onKey: () => false }),
  addProseMirrorPlugins() {
    const options = this.options
    return [new Plugin<number | null>({ key,
      state: { init: () => null, apply(tr, previous) {
        const meta = tr.getMeta(key) as { close?: boolean; from?: number } | undefined
        if (meta?.close) return null
        if (meta?.from != null) return meta.from
        if (previous == null) return null
        const mapped = tr.mapping.mapResult(previous, 1)
        return !mapped.deleted && matchAt(tr, mapped.pos) ? mapped.pos : null
      } },
      props: {
        handleTextInput(view, from, to, text) {
          const { $from, empty } = view.state.selection
          const before = $from.parent.textBetween(0, $from.parentOffset, '', '\ufffc')
          if (!view.editable || view.composing || text !== '@' || !empty || from !== to || !$from.parent.isTextblock ||
            $from.parent.type.spec.code || (view.state.storedMarks ?? $from.marks()).some(mark => ['code', 'link'].includes(mark.type.name)) ||
            (before && !/[\s([{“‘"']$/.test(before))) return false
          view.dispatch(view.state.tr.insertText('@', from, to).setMeta(key, { from }))
          return true
        },
        handleKeyDown: (view, event) => view.editable && !!referenceMatch(view.state) && !view.composing && !event.isComposing && options.onKey(event),
        attributes(state): Record<string, string> { return referenceMatch(state) ? { role: 'combobox', 'aria-expanded': 'true', 'aria-autocomplete': 'list', 'aria-controls': options.menuId, 'aria-haspopup': 'listbox' } : {} },
      },
      view(view) { let previous = ''; return { update() {
        const match = view.editable ? referenceMatch(view.state) : null
        const signature = JSON.stringify(match)
        if (signature !== previous) { previous = signature; options.onMatch(match) }
      }, destroy() { options.onMatch(null) } } },
    })]
  },
})
export function insertReferenceMatch(editor: Editor, doc: Y.Doc, offered: ReferenceMatch, id: string, boundary?: () => void): boolean {
  const current = referenceMatch(editor.state)
  const section = sectionReferenceIndex(doc).getSnapshot().find(section => section.key === id)
  if (!editor.isEditable || !section || !current || JSON.stringify(current) !== JSON.stringify(offered)) return false
  boundary?.()
  const inserted = editor.chain().focus().command(({ tr }) => { closeHistory(tr); return true }).insertContentAt({ from: current.from, to: current.to }, {
    type: 'sectionReference', attrs: { sectionId: id }, content: section.title ? [{ type: 'text', text: section.title }] : [],
  }).command(({ tr }) => { tr.setMeta(key, { close: true }); return true }).run()
  boundary?.()
  return inserted
}
