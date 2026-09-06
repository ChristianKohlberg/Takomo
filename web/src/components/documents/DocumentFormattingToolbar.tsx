import { useCallback, useSyncExternalStore } from 'react'
import type { Editor } from '@tiptap/react'
import { Bold, Italic, List, ListOrdered } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'

type Style = 'paragraph' | 'h1' | 'h2' | 'h3' | 'mixed' | 'other'
function currentStyle(editor: Editor): Style {
  const { selection, doc } = editor.state
  const styles = new Set<Style>()
  const add = (name: string, level?: number) => styles.add(name === 'paragraph' ? 'paragraph'
    : name === 'heading' && level && level <= 3 ? `h${level}` as Style : 'other')
  if (selection.empty) add(selection.$from.parent.type.name, selection.$from.parent.attrs.level)
  else doc.nodesBetween(selection.from, selection.to, node => {
    if (node.isTextblock) add(node.type.name, node.attrs.level)
  })
  return styles.size > 1 ? 'mixed' : [...styles][0] ?? 'other'
}
// Primitive snapshots avoid rerendering for typing that leaves formatting unchanged.
function snapshot(editor: Editor | null): string {
  if (!editor || editor.isDestroyed || !editor.isEditable) return ''
  const can = editor.can()
  return JSON.stringify({
    style: currentStyle(editor), paragraph: can.setParagraph(),
    h1: can.setHeading({ level: 1 }), h2: can.setHeading({ level: 2 }), h3: can.setHeading({ level: 3 }),
    bold: editor.isActive('bold'), italic: editor.isActive('italic'),
    bulletList: editor.isActive('bulletList'), orderedList: editor.isActive('orderedList'),
    canBold: can.toggleBold(), canItalic: can.toggleItalic(),
    canBulletList: can.toggleBulletList(), canOrderedList: can.toggleOrderedList(),
  })
}
type Formatting = {
  style: Style; paragraph: boolean; h1: boolean; h2: boolean; h3: boolean
  bold: boolean; italic: boolean; bulletList: boolean; orderedList: boolean
  canBold: boolean; canItalic: boolean; canBulletList: boolean; canOrderedList: boolean
}

/** Formats the current prose selection; structural section headings remain separate. */
export function DocumentFormattingToolbar({ editor, locale, canWrite }: {
  editor: Editor | null; locale: Locale; canWrite: boolean
}) {
  const subscribe = useCallback((notify: () => void) => {
    if (!editor || editor.isDestroyed) return () => {}
    editor.on('transaction', notify).on('selectionUpdate', notify).on('update', notify).on('destroy', notify)
    return () => { editor.off('transaction', notify).off('selectionUpdate', notify).off('update', notify).off('destroy', notify) }
  }, [editor])
  const getSnapshot = useCallback(() => snapshot(editor), [editor])
  const serialized = useSyncExternalStore(subscribe, getSnapshot, () => '')
  const state: Formatting | null = serialized ? JSON.parse(serialized) : null
  const de = locale === 'de'
  if (!canWrite) return null
  const format = (value: string) => {
    if (!editor || !editor.isEditable || editor.isDestroyed) return
    const command = editor.chain().focus()
    if (value === 'paragraph') command.setParagraph().run()
    else if (value === 'h1' || value === 'h2' || value === 'h3') command.setHeading({ level: Number(value[1]) as 1 | 2 | 3 }).run()
  }
  const controls = [
    { key: 'bold', label: de ? 'Fett' : 'Bold', Icon: Bold, active: state?.bold, allowed: state?.canBold, run: () => editor?.chain().focus().toggleBold().run() },
    { key: 'italic', label: de ? 'Kursiv' : 'Italic', Icon: Italic, active: state?.italic, allowed: state?.canItalic, run: () => editor?.chain().focus().toggleItalic().run() },
    { key: 'bullet', label: de ? 'Aufzählung' : 'Bulleted list', Icon: List, active: state?.bulletList, allowed: state?.canBulletList, run: () => editor?.chain().focus().toggleBulletList().run() },
    { key: 'ordered', label: de ? 'Nummerierte Liste' : 'Numbered list', Icon: ListOrdered, active: state?.orderedList, allowed: state?.canOrderedList, run: () => editor?.chain().focus().toggleOrderedList().run() },
  ]
  return <div className="flex min-w-0 flex-wrap items-center gap-1" role="group" aria-label={de ? 'Textformatierung' : 'Text formatting'}>
    <select aria-label={de ? 'Absatzstil' : 'Paragraph style'}
      className="h-7 max-w-full rounded-md border border-input bg-card px-2 text-xs text-foreground focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-50"
      value={state?.style ?? ''} disabled={!state} onChange={event => format(event.target.value)}>
      {!state && <option value="">{de ? 'Text auswählen' : 'Select text'}</option>}
      {state?.style === 'mixed' && <option value="mixed" disabled>{de ? 'Gemischt' : 'Mixed styles'}</option>}
      {state?.style === 'other' && <option value="other" disabled>{de ? 'Anderer Stil' : 'Other style'}</option>}
      <option value="paragraph" disabled={!state?.paragraph}>{de ? 'Normaler Text' : 'Normal text'}</option>
      <option value="h1" disabled={!state?.h1}>{de ? 'Überschrift 1' : 'Heading 1'}</option>
      <option value="h2" disabled={!state?.h2}>{de ? 'Überschrift 2' : 'Heading 2'}</option>
      <option value="h3" disabled={!state?.h3}>{de ? 'Überschrift 3' : 'Heading 3'}</option>
    </select>
    {controls.map(({ key, label, Icon, active, allowed, run }) => <Button key={key} variant="ghost" size="icon-sm"
      aria-label={label} title={label} aria-pressed={!!active} disabled={!state || !allowed}
      onMouseDown={event => event.preventDefault()} onClick={() => { if (editor?.isEditable && !editor.isDestroyed) run() }}>
      <Icon className="size-3.5" aria-hidden="true" />
    </Button>)}
  </div>
}
