import { useId, useRef, useState } from 'react'
import { EditorContent, useEditor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import { SlashInsert, type SlashMatch } from '@/lib/slash-insert'
import type { Locale } from '@/lib/i18n'
import { SlashMenu } from './SlashMenu'

/** A local draft: opening an empty document must never create shared content. */
export function DocumentStart({ locale, onInsert }: {
  locale: Locale
  onInsert: (level: 1 | 2 | 3, title: string) => boolean
}) {
  const menuId = useId()
  const hintId = useId()
  const keys = useRef<((event: KeyboardEvent) => boolean) | null>(null)
  const [match, setMatch] = useState<SlashMatch | null>(null)
  const editor = useEditor({
    extensions: [StarterKit, SlashInsert.configure({ menuId, onMatch: setMatch, onKey: event => keys.current?.(event) ?? false })],
    immediatelyRender: false,
    editorProps: { attributes: {
      class: 'min-h-12 px-1 py-2 focus:outline-none',
      'aria-label': locale === 'de' ? 'Erster Abschnitt' : 'First section',
      'aria-describedby': hintId,
    } },
  })
  return <div className="document-page mx-auto">
    <p id={hintId} className="text-muted-foreground text-sm">{locale === 'de'
      ? 'Schreibe /h1 Titel und drücke Enter, um dein Dokument zu beginnen.'
      : 'Type /h1 Title and press Enter to start your document.'}</p>
    <EditorContent editor={editor} />
    {editor && match && <SlashMenu key={match.query} editor={editor} match={match} locale={locale} menuId={menuId}
      keys={keys} onInsertSection={onInsert} maxSectionLevel={1} sectionsOnly />}
  </div>
}
