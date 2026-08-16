import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { Anchor } from '@/lib/initiative-anchor'
import type { Amendment, Thread } from '@/lib/initiative-doc'
import type { Entry } from '@/lib/initiatives'
import { cn } from '@/lib/utils'

/** The four things a reader can do with a passage they highlighted. */
export type Operation = 'comment' | 'suggest' | 'ticket' | 'ask' | 'cite'

export interface SelectionPaneLabels {
  idleHeading: string
  idleHint: string
  selectionHeading: string
  comment: string
  suggest: string
  ticket: string
  ask: string
  cite: string
  commentPh: string
  suggestPh: string
  ticketPh: string
  askPh: string
  citePh: string
  submit: string
  cancel: string
  working: string
  openNotes: string
  pendingSuggestions: string
  noteBy: string
  dispatch: string
  accept: string
  reject: string
  resolve: string
  threadOpen: string
  threadRunning: string
  threadResolved: string
  orphanWarning: string
  ticketMade: string
  replaces: string
  with: string
  readOnly: string
  noEvidence: string
}

export interface SelectionPaneProps {
  labels: SelectionPaneLabels
  canWrite: boolean
  busy: boolean
  /** The live highlight, or null when nothing is selected. */
  anchor: Anchor | null
  /** The note or suggestion opened by clicking a highlight. */
  focused: { thread: Thread } | { amendment: Amendment } | null
  /** Everything still awaiting somebody, shown when nothing is selected. */
  openThreads: Thread[]
  pending: Amendment[]
  /** Entries citable as evidence — anything that is not part of the document. */
  evidence: Entry[]
  onRun: (op: Operation, text: string, evidenceId?: string) => void
  onOpenThread: (t: Thread) => void
  onOpenAmendment: (a: Amendment) => void
  onDispatch: (t: Thread) => void
  onResolve: (t: Thread) => void
  onAccept: (a: Amendment) => void
  onReject: (a: Amendment) => void
  onDismiss: () => void
}

/**
 * The right-hand pane: what you can do, and what is already outstanding.
 *
 * Three states, and which one is showing is decided entirely by props — a live
 * highlight beats an opened note, which beats the idle list. Keeping that
 * precedence here rather than in the page is what stops a stale note from
 * lingering beside a fresh selection.
 */
export function SelectionPane(props: SelectionPaneProps) {
  const { labels, anchor, focused, canWrite } = props
  const [op, setOp] = useState<Operation | null>(null)
  const [text, setText] = useState('')
  const [evidenceId, setEvidenceId] = useState('')

  // A new highlight is a new question being asked, so the half-typed answer to
  // the previous one must not be carried into it.
  useEffect(() => {
    setOp(null)
    setText('')
    setEvidenceId('')
  }, [anchor?.quote, anchor?.para, anchor?.pane])

  if (anchor) {
    return (
      <Shell heading={labels.selectionHeading} onDismiss={props.onDismiss}>
        <blockquote className="border-primary text-foreground my-2 border-l-2 pl-3 text-[13.5px] italic">
          {anchor.quote}
        </blockquote>

        {!canWrite ? (
          <p className="text-muted-foreground text-[12.5px]">{labels.readOnly}</p>
        ) : op === null ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {(['comment', 'suggest', 'ticket', 'ask', 'cite'] as const).map((o) => (
              <Button
                key={o}
                variant="outline"
                size="sm"
                onClick={() => {
                  setOp(o)
                  // Suggesting starts from the words being replaced: editing
                  // them is the gesture, and an empty box would invite deleting
                  // the passage by accident.
                  setText(o === 'suggest' ? anchor.quote : '')
                }}
              >
                {labels[o]}
              </Button>
            ))}
          </div>
        ) : (
          <form
            className="mt-3"
            onSubmit={(e) => {
              e.preventDefault()
              props.onRun(op, text.trim(), evidenceId || undefined)
            }}
          >
            {op === 'cite' ? (
              props.evidence.length === 0 ? (
                <p className="text-muted-foreground text-[12.5px]">{labels.noEvidence}</p>
              ) : (
                <select
                  aria-label={labels.cite}
                  value={evidenceId}
                  onChange={(e) => setEvidenceId(e.target.value)}
                  className="border-border bg-card text-foreground w-full rounded-md border px-2 py-1.5 text-[13px]"
                >
                  <option value="">{labels.citePh}</option>
                  {props.evidence.map((e) => (
                    <option key={e.id} value={e.id}>
                      {e.kind} — {e.title || e.text?.slice(0, 60) || e.id}
                    </option>
                  ))}
                </select>
              )
            ) : (
              <Textarea
                autoFocus
                rows={op === 'suggest' ? 4 : 3}
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder={
                  op === 'comment'
                    ? labels.commentPh
                    : op === 'suggest'
                      ? labels.suggestPh
                      : op === 'ticket'
                        ? labels.ticketPh
                        : labels.askPh
                }
                className="text-[13px]"
              />
            )}
            <div className="mt-2 flex gap-1.5">
              <Button
                type="submit"
                size="sm"
                disabled={props.busy || (op === 'cite' ? !evidenceId : !text.trim())}
              >
                {props.busy ? labels.working : labels.submit}
              </Button>
              <Button type="button" variant="ghost" size="sm" onClick={() => setOp(null)}>
                {labels.cancel}
              </Button>
            </div>
          </form>
        )}
      </Shell>
    )
  }

  if (focused && 'thread' in focused) {
    const t = focused.thread
    return (
      <Shell heading={labels.openNotes} onDismiss={props.onDismiss}>
        <NoteBody thread={t} labels={labels} />
        {canWrite && t.state === 'open' && (
          <div className="mt-3 flex gap-1.5">
            <Button size="sm" disabled={props.busy} onClick={() => props.onDispatch(t)}>
              {labels.dispatch}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={props.busy}
              onClick={() => props.onResolve(t)}
            >
              {labels.resolve}
            </Button>
          </div>
        )}
      </Shell>
    )
  }

  if (focused && 'amendment' in focused) {
    const a = focused.amendment
    return (
      <Shell heading={labels.pendingSuggestions} onDismiss={props.onDismiss}>
        <SuggestionBody amendment={a} labels={labels} />
        {canWrite && (
          <div className="mt-3 flex gap-1.5">
            <Button size="sm" disabled={props.busy || a.orphaned} onClick={() => props.onAccept(a)}>
              {labels.accept}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={props.busy}
              onClick={() => props.onReject(a)}
            >
              {labels.reject}
            </Button>
          </div>
        )}
      </Shell>
    )
  }

  return (
    <Shell heading={labels.idleHeading}>
      <p className="text-muted-foreground mt-0 text-[12.5px]">{labels.idleHint}</p>

      {props.pending.length > 0 && (
        <>
          <Heading>{labels.pendingSuggestions}</Heading>
          {props.pending.map((a) => (
            <Row key={a.entry.id} onClick={() => props.onOpenAmendment(a)}>
              <SuggestionBody amendment={a} labels={labels} compact />
            </Row>
          ))}
        </>
      )}

      {props.openThreads.length > 0 && (
        <>
          <Heading>{labels.openNotes}</Heading>
          {props.openThreads.map((t) => (
            <Row key={t.entry.id} onClick={() => props.onOpenThread(t)}>
              <NoteBody thread={t} labels={labels} compact />
            </Row>
          ))}
        </>
      )}
    </Shell>
  )
}

function Shell({
  heading,
  onDismiss,
  children,
}: {
  heading: string
  onDismiss?: () => void
  children: React.ReactNode
}) {
  return (
    <div className="p-4">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-muted-foreground m-0 text-[11.5px] font-bold tracking-[0.08em] uppercase">
          {heading}
        </h2>
        {onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Close"
            className="text-muted-foreground hover:text-foreground cursor-pointer text-[16px] leading-none"
          >
            ×
          </button>
        )}
      </div>
      {children}
    </div>
  )
}

function Heading({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-muted-foreground mt-4 mb-1 text-[11px] font-bold tracking-[0.06em] uppercase">
      {children}
    </p>
  )
}

function Row({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="border-border hover:bg-muted mb-1.5 block w-full cursor-pointer rounded-md border p-2 text-left"
    >
      {children}
    </button>
  )
}

function NoteBody({
  thread,
  labels,
  compact,
}: {
  thread: Thread
  labels: SelectionPaneLabels
  compact?: boolean
}) {
  const state =
    thread.state === 'running'
      ? labels.threadRunning
      : thread.state === 'resolved'
        ? labels.threadResolved
        : labels.threadOpen
  return (
    <>
      {thread.anchor && (
        <blockquote
          className={cn(
            'border-border text-muted-foreground my-1 border-l-2 pl-2 text-[12px] italic',
            thread.orphaned && 'line-through',
          )}
        >
          {thread.anchor.quote}
        </blockquote>
      )}
      {thread.orphaned && (
        <p className="text-muted-foreground my-1 text-[11.5px]">{labels.orphanWarning}</p>
      )}
      <p className={cn('text-foreground my-1', compact ? 'text-[12.5px]' : 'text-[13.5px]')}>
        {thread.entry.text}
      </p>
      <p className="text-muted-foreground m-0 font-mono text-[11px]">
        {labels.noteBy} {thread.entry.source} · {state}
        {thread.ticket ? ` · ${labels.ticketMade} ${thread.ticket}` : ''}
      </p>
    </>
  )
}

function SuggestionBody({
  amendment,
  labels,
  compact,
}: {
  amendment: Amendment
  labels: SelectionPaneLabels
  compact?: boolean
}) {
  if (amendment.scope === 'pane') {
    return (
      <>
        <p className={cn('text-foreground my-1', compact ? 'text-[12.5px]' : 'text-[13.5px]')}>
          {amendment.diff.filter((d) => d.kind !== 'same').length} ¶
        </p>
        {amendment.diff
          .filter((d) => d.kind !== 'same')
          .map((d, i) => (
            <p key={i} className="text-muted-foreground my-0.5 text-[12px]">
              <span className="font-mono">{d.kind}</span> {d.text}
            </p>
          ))}
        <p className="text-muted-foreground m-0 font-mono text-[11px]">
          {labels.noteBy} {amendment.entry.source}
        </p>
      </>
    )
  }
  return (
    <>
      <p className="text-muted-foreground m-0 text-[11px]">{labels.replaces}</p>
      <blockquote
        className={cn(
          'border-border text-muted-foreground my-1 border-l-2 pl-2 text-[12px] italic',
          amendment.orphaned && 'line-through',
        )}
      >
        {amendment.anchor?.quote}
      </blockquote>
      <p className="text-muted-foreground m-0 text-[11px]">{labels.with}</p>
      <p className={cn('text-foreground my-1', compact ? 'text-[12.5px]' : 'text-[13.5px]')}>
        {amendment.replacement}
      </p>
      {amendment.orphaned && (
        <p className="text-muted-foreground my-1 text-[11.5px]">{labels.orphanWarning}</p>
      )}
      <p className="text-muted-foreground m-0 font-mono text-[11px]">
        {labels.noteBy} {amendment.entry.source}
      </p>
    </>
  )
}
