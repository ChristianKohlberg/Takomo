import { useEffect, useState } from 'react'
import type { Editor } from '@tiptap/react'
import { MessageSquarePlus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { captureCommentAnchor, MAX_COMMENT_LENGTH, type CommentAnchor } from '@/lib/document-comments'
import type { Locale } from '@/lib/i18n'

export function DocumentCommentButton({ editor, canWrite, locale, onComment }: {
  editor: Editor | null; canWrite: boolean; locale: Locale; onComment: (anchor: CommentAnchor) => void
}) {
  const [, refresh] = useState(0)
  useEffect(() => {
    const update = () => refresh(n => n + 1)
    editor?.on('selectionUpdate', update)
    editor?.on('transaction', update)
    return () => { editor?.off('selectionUpdate', update); editor?.off('transaction', update) }
  }, [editor])
  if (!canWrite) return null
  const selection = editor?.state.selection
  const text = editor && selection ? editor.state.doc.textBetween(selection.from, selection.to, '\n') : ''
  const available = Boolean(editor && !editor.isDestroyed && text.trim() && text.length <= MAX_COMMENT_LENGTH)
  return <Button variant="ghost" size="sm" disabled={!available} aria-label={locale === 'de' ? 'Kommentieren' : 'Add comment'} title={locale === 'de' ? 'Text auswählen und kommentieren' : 'Select text to comment'} onMouseDown={event => event.preventDefault()} onClick={() => {
    if (!editor || editor.isDestroyed) return
    const anchor = captureCommentAnchor(editor)
    if (anchor) onComment(anchor)
  }}><MessageSquarePlus className="size-3.5" aria-hidden="true" /><span className="hidden sm:inline">{locale === 'de' ? 'Kommentieren' : 'Add comment'}</span></Button>
}
