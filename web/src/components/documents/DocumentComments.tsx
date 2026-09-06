import { useEffect, useState } from 'react'
import type { Editor } from '@tiptap/react'
import type * as Y from 'yjs'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { COMMENT_FIELD, MAX_COMMENT_LENGTH, createCommentThread, readCommentThreads, replyToComment, resolveCommentAnchor, resolveCommentThread, type CommentAnchor } from '@/lib/document-comments'

export interface DocumentCommentsProps {
  ydoc: Y.Doc; sectionId: string; editor: Editor | null; actor: string; canWrite: boolean; locale: Locale
  draft?: CommentAnchor | null; onDraftConsumed: () => void; onClose: () => void
}
export function DocumentComments({ ydoc, sectionId, editor, actor, canWrite, locale, draft, onDraftConsumed, onClose }: DocumentCommentsProps) {
  const de = locale === 'de'
  const [, refresh] = useState(0)
  const [text, setText] = useState('')
  const [error, setError] = useState('')
  useEffect(() => {
    const comments = ydoc.getMap(COMMENT_FIELD)
    const update = () => refresh(n => n + 1)
    comments.observeDeep(update)
    editor?.on('transaction', update)
    return () => { comments.unobserveDeep(update); editor?.off('transaction', update) }
  }, [ydoc, editor])
  const threads = readCommentThreads(ydoc, sectionId)
  const attempt = (operation: () => void) => {
    if (!canWrite) return false
    try { operation(); setError(''); return true } catch { setError(de ? 'Kommentar konnte nicht gespeichert werden. Bitte erneut versuchen.' : 'Could not save the comment. Please try again.'); return false }
  }
  return <section className="min-w-0 border-b border-border-soft bg-card p-3" aria-label={de ? 'Textkommentare' : 'Text comments'}>
    <div className="flex items-center justify-between gap-2"><h2 className="font-medium">{de ? 'Textkommentare' : 'Text comments'}</h2><Button variant="ghost" size="sm" onClick={onClose}>{de ? 'Schließen' : 'Close comments'}</Button></div>
    {error && <p role="alert">{error}</p>}
    {draft && canWrite && <form className="mt-2 space-y-2" onSubmit={event => { event.preventDefault(); attempt(() => {
      createCommentThread(ydoc, sectionId, draft, actor, text)
      setText(''); onDraftConsumed()
    }) }}>
      <blockquote className="max-h-28 overflow-auto border-l-2 border-border pl-2 text-sm break-words">{draft.quote}</blockquote>
      <textarea autoFocus aria-label={de ? 'Neuer Kommentar' : 'New comment'} className="w-full min-w-0 rounded border border-border bg-background p-2 text-sm" rows={2} maxLength={MAX_COMMENT_LENGTH} value={text} onChange={event => setText(event.target.value)} />
      <div className="flex gap-2"><Button type="submit" size="sm" disabled={!text.trim()}>{de ? 'Kommentieren' : 'Post comment'}</Button><Button type="button" variant="ghost" size="sm" onClick={() => { setText(''); onDraftConsumed() }}>{de ? 'Abbrechen' : 'Cancel'}</Button></div>
    </form>}
    {!draft && threads.length === 0 && <p className="mt-2 text-sm text-muted-foreground">{canWrite ? (de ? 'Text auswählen, um einen Kommentar hinzuzufügen.' : 'Select text to add a comment.') : (de ? 'Noch keine Kommentare.' : 'No comments yet.')}</p>}
    <div className="max-h-80 space-y-3 overflow-auto">
      {threads.map(thread => {
        const range = editor ? resolveCommentAnchor(editor, thread.anchor) : null
        return <article key={thread.id} className="mt-3 rounded border border-border-soft p-3" aria-label={de ? 'Kommentarthread' : 'Comment thread'}>
          <blockquote className="border-l-2 border-border pl-2 text-sm break-words">{thread.anchor.quote}</blockquote>
          <div className="my-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span>{thread.resolved ? (de ? 'Erledigt' : 'Resolved') : (de ? 'Offen' : 'Open')}</span>
            {editor && !range && <span>{de ? 'Text geändert oder entfernt · Zitat erhalten' : 'Text changed or removed · quote retained'}</span>}
            {range && <button className="underline" onClick={() => { editor?.commands.setTextSelection(range); editor?.commands.focus(); editor?.commands.scrollIntoView() }}>{de ? 'Text anzeigen' : 'Show text'}</button>}
          </div>
          {thread.messages.map(m => <div key={m.id} className="mt-2 text-sm"><span className="font-medium">{m.author}</span><p className="whitespace-pre-wrap break-words">{m.text}</p></div>)}
          {canWrite && <>
            <Button variant="ghost" size="sm" onClick={() => attempt(() => resolveCommentThread(ydoc, thread.id, !thread.resolved))}>{thread.resolved ? (de ? 'Wieder öffnen' : 'Reopen') : (de ? 'Erledigen' : 'Resolve')}</Button>
            {!thread.resolved && <ReplyForm locale={locale} onReply={reply => attempt(() => replyToComment(ydoc, thread.id, actor, reply))} />}
          </>}
        </article>
      })}
    </div>
  </section>
}
function ReplyForm({ locale, onReply }: { locale: Locale; onReply: (text: string) => boolean }) {
  const [text, setText] = useState('')
  return <form className="mt-2 flex min-w-0 flex-wrap gap-2" onSubmit={event => { event.preventDefault(); if (onReply(text)) setText('') }}>
    <textarea aria-label={locale === 'de' ? 'Antwort' : 'Reply'} className="min-w-0 flex-1 rounded border border-border bg-background p-2 text-sm" rows={1} maxLength={MAX_COMMENT_LENGTH} value={text} onChange={event => setText(event.target.value)} />
    <Button type="submit" size="sm" disabled={!text.trim()}>{locale === 'de' ? 'Antworten' : 'Reply'}</Button>
  </form>
}
