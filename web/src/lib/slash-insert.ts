import { Extension, type Editor } from '@tiptap/react'
import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state'

export interface SlashMatch { from: number; to: number; query: string }
const slashKey = new PluginKey<number | null>('slashInsert')

export function slashMatch(state: EditorState): SlashMatch | null {
  const from = slashKey.getState(state)
  if (from == null || from > state.doc.content.size) return null
  const { $from, empty, to } = state.selection
  if (!empty || $from.depth !== 1 || $from.parent.type.name !== 'paragraph' || $from.start() !== from) return null
  const text = $from.parent.textContent
  if (!text.startsWith('/') || text.length > 80 || to !== from + $from.parent.content.size) return null
  return { from, to, query: text.slice(1) }
}

/** Only a locally typed slash in an empty top-level paragraph opens the menu. */
export const SlashInsert = Extension.create<{
  menuId: string
  onMatch: (match: SlashMatch | null) => void
  onKey: (event: KeyboardEvent) => boolean
}>({
  name: 'slashInsert',
  addOptions: () => ({ menuId: 'slash-insert', onMatch: () => {}, onKey: () => false }),
  addProseMirrorPlugins() {
    const options = this.options
    return [new Plugin<number | null>({
      key: slashKey,
      state: {
        init: () => null,
        apply(tr, previous) {
          const meta = tr.getMeta(slashKey) as { from?: number; close?: boolean } | undefined
          if (meta?.close) return null
          if (meta?.from != null) return meta.from
          if (previous == null) return null
          const mapped = tr.mapping.mapResult(previous, -1)
          if (mapped.deleted) return null
          const { $from, empty, to } = tr.selection
          if (!empty || $from.depth !== 1 || $from.parent.type.name !== 'paragraph' || $from.start() !== mapped.pos ||
            !$from.parent.textContent.startsWith('/') || to !== mapped.pos + $from.parent.content.size) return null
          return mapped.pos
        },
      },
      props: {
        handleTextInput(view, from, to, text) {
          const { $from, empty } = view.state.selection
          if (!view.editable || view.composing || text !== '/' || !empty || from !== to ||
            $from.depth !== 1 || $from.parent.type.name !== 'paragraph' || $from.parent.content.size) return false
          view.dispatch(view.state.tr.insertText(text, from, to).setMeta(slashKey, { from }))
          return true
        },
        handleKeyDown(view, event) {
          return !!slashMatch(view.state) && !view.composing && options.onKey(event)
        },
        attributes(state): Record<string, string> {
          return slashMatch(state) ? {
            role: 'combobox', 'aria-expanded': 'true', 'aria-autocomplete': 'list',
            'aria-controls': options.menuId, 'aria-haspopup': 'listbox',
          } : {}
        },
      },
      view(view) {
        let last = ''
        return { update() {
          const match = view.editable ? slashMatch(view.state) : null
          const signature = JSON.stringify(match)
          if (signature !== last) { last = signature; options.onMatch(match) }
        }, destroy() { options.onMatch(null) } }
      },
    })]
  },
})

export function closeSlashMenu(editor: Editor) {
  editor.view.dispatch(editor.state.tr.setMeta(slashKey, { close: true }))
}

export type InsertKind = 'heading1' | 'heading2' | 'heading3' | 'bulletList' | 'orderedList' | 'quote' | 'code' | 'table' | 'mermaid'

/** A stale menu may never replace somebody else's text. */
export function insertSlashBlock(editor: Editor, match: SlashMatch, kind: InsertKind, rows = 3, cols = 3): boolean {
  const current = slashMatch(editor.state)
  if (!editor.isEditable || !current || current.from !== match.from || current.to !== match.to || current.query !== match.query) return false
  if (kind === 'table' && (!Number.isInteger(rows) || !Number.isInteger(cols) || rows < 1 || cols < 1 || rows > 10 || cols > 10)) return false
  const chain = editor.chain().focus().deleteRange({ from: match.from, to: match.to })
    .command(({ tr }) => { tr.setMeta(slashKey, { close: true }); return true })
  switch (kind) {
    case 'heading1': return chain.setHeading({ level: 1 }).run()
    case 'heading2': return chain.setHeading({ level: 2 }).run()
    case 'heading3': return chain.setHeading({ level: 3 }).run()
    case 'bulletList': return chain.toggleBulletList().run()
    case 'orderedList': return chain.toggleOrderedList().run()
    case 'quote': return chain.setBlockquote().run()
    case 'code': return chain.setCodeBlock().run()
    case 'mermaid': return chain.setCodeBlock({ language: 'mermaid' }).run()
    case 'table': return chain.insertTable({ rows, cols, withHeaderRow: true }).run()
  }
}
