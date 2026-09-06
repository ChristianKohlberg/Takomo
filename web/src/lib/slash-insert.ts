import { Extension, type Editor } from '@tiptap/react'
// Include the command augmentations when this helper is emitted alone in the
// component library, without importing the page's editor at runtime.
import type {} from '@tiptap/starter-kit'
import type {} from '@tiptap/extension-table'
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
          return view.editable && !!slashMatch(view.state) && !view.composing && !event.isComposing && options.onKey(event)
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

export type InsertKind = 'heading1' | 'heading2' | 'heading3' | 'bulletList' | 'orderedList' | 'quote' | 'code' | 'table' | 'mermaid' | 'plantuml' | 'd2' | 'wireframe'

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
    case 'plantuml': return chain.setCodeBlock({ language: 'plantuml' }).insertContent('@startuml\nAlice -> Bob: Hello\n@enduml').run()
    case 'd2': return chain.setCodeBlock({ language: 'd2' }).insertContent('User -> Takomo: Request\nTakomo -> Worker: Process').run()
    case 'wireframe': return chain.setCodeBlock({ language: 'plantuml' }).insertContent('@startsalt\n{\n  Settings\n  Server | "https://takomo.example"\n  [Connect]\n}\n@endsalt').run()
    case 'table': return chain.insertTable({ rows, cols, withHeaderRow: true }).run()
  }
}

/** Create the canonical section first; refusal leaves the slash query recoverable.
 * Do not focus this editor: the section owner focuses the newly created body. */
export function insertSlashSection(editor: Editor, match: SlashMatch, level: 1 | 2 | 3, title: string,
  insert: (level: 1 | 2 | 3, title: string) => boolean): boolean {
  const current = slashMatch(editor.state)
  if (!title.trim() || !editor.isEditable || !current || current.from !== match.from || current.to !== match.to || current.query !== match.query) return false
  if (!insert(level, title.trim())) return false
  editor.view.dispatch(editor.state.tr.delete(match.from, match.to).setMeta(slashKey, { close: true }))
  return true
}
