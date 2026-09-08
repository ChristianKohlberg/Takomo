import { useEffect, useState } from 'react'
import type { Editor } from '@tiptap/react'
import type * as Y from 'yjs'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import { COMMENT_FIELD, MAX_COMMENT_LENGTH, createCommentThread, readCommentThreads, replyToComment, resolveCommentAnchor, resolveCommentThread, type CommentAnchor, type CommentThread } from '@/lib/document-comments'

export interface DocumentCommentsProps {
  ydoc: Y.Doc; sectionId?: string; editor: Editor | null; actor: string; canWrite: boolean; locale: Locale
  sectionTitle?: (id: string) => string | null; onShowThread?: (thread: CommentThread) => void;
  draft?: CommentAnchor | null; onDraftConsumed: () => void; onClose: () => void
}
export function DocumentComments({ ydoc, sectionId, editor, actor, canWrite, locale, draft, onDraftConsumed, onClose, sectionTitle, onShowThread }: DocumentCommentsProps) {
  const de = locale === 'de'
  const [, refresh] = useState(0)
  const [text, setText] = useState('')
  const [error, setError] = useState('')
  const [filter, setFilter] = useState('open')
  const global = sectionId === undefined
  useEffect(() => {
    const comments = ydoc.getMap(COMMENT_FIELD)
    const update = () => refresh(n => n + 1)
    comments.observeDeep(update)
    editor?.on('transaction', update)
    return () => { comments.unobserveDeep(update); editor?.off('transaction', update) }
  }, [ydoc, editor])
  const allThreads = readCommentThreads(ydoc, sectionId)
  const threads = allThreads.filter(thread => !global || filter === 'all' || thread.resolved === (filter === 'resolved'))
  const attempt = (operation: () => void) => {
    if (!canWrite) return false
    try { operation(); setError(''); return true } catch { setError(de ? 'Kommentar konnte nicht gespeichert werden. Bitte erneut versuchen.' : 'Could not save the comment. Please try again.'); return false }
  }
  return <section className="min-w-0 border-b border-border-soft bg-card p-3" aria-label={global ? (de ? 'Dokumentkommentare' : 'Document comments') : (de ? 'Textkommentare' : 'Text comments')}>
    <div className="flex items-center justify-between gap-2"><h2 className="font-medium">{global ? (de ? 'Dokumentkommentare' : 'Document comments') : (de ? 'Textkommentare' : 'Text comments')}</h2><Button variant="ghost" size="sm" onClick={onClose}>{de ? 'Schließen' : 'Close comments'}</Button></div>
    {global && <select aria-label={de ? 'Kommentare filtern' : 'Filter comments'} value={filter} onChange={event => setFilter(event.target.value)} className="my-2 rounded border border-border bg-background px-2 py-1 text-sm"><option value="open">{de ? 'Offen' : 'Open'} ({allThreads.filter(t => !t.resolved).length})</option><option value="resolved">{de ? 'Erledigt' : 'Resolved'} ({allThreads.filter(t => t.resolved).length})</option><option value="all">{de ? 'Alle' : 'All'} ({allThreads.length})</option></select>}
    {error && <p role="alert">{error}</p>}
    {draft && canWrite && sectionId && <form className="mt-2 space-y-2" onSubmit={event => { event.preventDefault(); attempt(() => {
      createCommentThread(ydoc, sectionId, draft, actor, text)
      setText(''); onDraftConsumed()
    }) }}>
      <blockquote className="max-h-28 overflow-auto border-l-2 border-border pl-2 text-sm break-words">{draft.quote}</blockquote>
      <textarea autoFocus aria-label={de ? 'Neuer Kommentar' : 'New comment'} className="w-full min-w-0 rounded border border-border bg-background p-2 text-sm" rows={2} maxLength={MAX_COMMENT_LENGTH} value={text} onChange={event => setText(event.target.value)} />
      <div className="flex gap-2"><Button type="submit" size="sm" disabled={!text.trim()}>{de ? 'Kommentieren' : 'Post comment'}</Button><Button type="button" variant="ghost" size="sm" onClick={() => { setText(''); onDraftConsumed() }}>{de ? 'Abbrechen' : 'Cancel'}</Button></div>
    </form>}
    {!draft && threads.length === 0 && <p className="mt-2 text-sm text-muted-foreground">{global ? (allThreads.length ? (de ? 'Keine Kommentare für diesen Filter.' : 'No comments for this filter.') : canWrite ? (de ? 'Noch keine Kommentare. Wähle Text im Dokument und dann Kommentar hinzufügen.' : 'No comments yet. Select text in the document, then Add comment.') : (de ? 'Noch keine Kommentare.' : 'No comments yet.')) : canWrite ? (de ? 'Text auswählen, um einen Kommentar hinzuzufügen.' : 'Select text to add a comment.') : (de ? 'Noch keine Kommentare.' : 'No comments yet.')}</p>}
    <div className={global ? "space-y-3" : "max-h-80 space-y-3 overflow-auto"}>
      {threads.map(thread => {
        const range = editor ? resolveCommentAnchor(editor, thread.anchor) : null
        return <article key={thread.id} className="mt-3 rounded border border-border-soft p-3" aria-label={de ? 'Kommentarthread' : 'Comment thread'}>
          {global && <p className="mb-2 text-sm font-medium break-words">{sectionTitle?.(thread.sectionId) ?? (de ? 'Abschnitt entfernt' : 'Section removed')}</p>}
          <blockquote className="border-l-2 border-border pl-2 text-sm break-words">{thread.anchor.quote}</blockquote>
          <div className="my-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
            <span>{thread.resolved ? (de ? 'Erledigt' : 'Resolved') : (de ? 'Offen' : 'Open')}</span>
            {editor && !range && <span>{de ? 'Text geändert oder entfernt · Zitat erhalten' : 'Text changed or removed · quote retained'}</span>}
            {global && sectionTitle?.(thread.sectionId) !== null && <button className="underline" onClick={() => onShowThread?.(thread)}>{de ? 'Zum Text' : 'Go to text'}</button>}
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
