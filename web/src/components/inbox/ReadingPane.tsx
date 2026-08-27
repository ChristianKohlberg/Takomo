// The reading pane: the question in full, its follow-up thread, the answer
// control, and the primary.
//
// The primary is deliberately NOT the native `disabled` attribute: that drops
// the control out of the tab order and explains nothing, which is the same
// dead-control trap as a silently inert button. `aria-disabled` keeps it
// focusable and announces it as unavailable, the visible hint carries the reason
// for everyone, and pressing it anyway repeats that reason.
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Markdown } from '@/components/Markdown'
import { AnswerArea, type AnswerAreaLabels } from './AnswerArea'
import { cn } from '@/lib/utils'
import { fmtAge } from '@/lib/format'
import { answerBlockReason, type Draft } from '@/lib/answers'
import type { Question, ThreadMessage } from '@/lib/questions'
import { Hint } from '@/components/Hint'
import { Picker } from '@/components/Picker'

/** A thread reply can run to thousands of characters — clamp the preview. */
const THREAD_MSG_CLAMP_CHARS = 700
const THREAD_MSG_CLAMP_LINES = 12

function threadNeedsClamp(body: string): boolean {
  return body.length > THREAD_MSG_CLAMP_CHARS || body.split('\n').length > THREAD_MSG_CLAMP_LINES
}

function ThreadBody({
  body,
  expanded,
  onToggle,
  labels,
}: {
  body: string
  expanded: boolean
  onToggle: () => void
  labels: { msgMore: string; msgLess: string }
}) {
  const clamp = threadNeedsClamp(body)
  if (!clamp) return <Markdown text={body} className="text-[13px]" />
  return (
    <div>
      <div
        className={cn(
          !expanded &&
            'max-h-[13em] overflow-hidden [mask-image:linear-gradient(to_bottom,#000_72%,transparent)] [-webkit-mask-image:linear-gradient(to_bottom,#000_72%,transparent)]',
        )}
      >
        <Markdown text={body} className="text-[13px]" />
      </div>
      <button
        type="button"
        className="text-primary mt-1 cursor-pointer text-[12px] font-[650] underline underline-offset-2"
        aria-expanded={expanded}
        onClick={onToggle}
      >
        {expanded ? labels.msgLess : labels.msgMore}
      </button>
    </div>
  )
}

export interface ReadingPaneLabels extends AnswerAreaLabels {
  submit: string
  sendFollow: string
  askFollow: string
  followFirst: string
  to: string
  typeFirst: string
  sendFirst: string
  share: string
  withdraw: string
  reopen: string
  closed: string
  advisory: string
  askedBy: string
  readonly: string
  /** Back to the question list; narrow screens only. */
  back?: string
  waitingAgentPrefix: string
  waitingAgentSuffix: string
  noReply: string
  /** The assignee control: "Waiting on", its empty option, and its tooltip. */
  assignTo: string
  assignNobody: string
  assignHint: string
  /** Long thread replies: expand/collapse toggle labels. */
  msgMore: string
  msgLess: string
}

export interface ReadingPaneProps {
  question: Question
  thread: ThreadMessage[]
  draft: Draft | undefined
  onDraft: (patch: Draft) => void
  canAnswer: boolean
  labels: ReadingPaneLabels
  onSubmit: () => void
  onFollowup: (text: string) => void
  onWithdraw: () => void
  onReopen: () => void
  /** Shown only on narrow screens, where the pane replaces the list. */
  onBack?: () => void
  /** Lets the page hide the pane on narrow screens when nothing is chosen. */
  className?: string
  onShare: () => void
  /** Optimistic answer still in the undo window — Reopen must stay hidden. */
  answerPending?: boolean
  /**
   * People this question can be addressed to: the members of its project. Empty
   * (or absent) hides the control — on an instance with no directory, or for a
   * reader who cannot assign, there is nothing to offer.
   */
  assignable?: { handle: string; label: string }[]
  /** `null` returns the question to the queue. */
  onAssign?: (handle: string | null) => void
}

export function ReadingPane({
  onBack,
  className,
  question: q,
  thread,
  draft,
  onDraft,
  canAnswer,
  labels,
  onSubmit,
  onFollowup,
  onWithdraw,
  onReopen,
  onShare,
  answerPending = false,
  assignable,
  onAssign,
}: ReadingPaneProps) {
  const [composing, setComposing] = useState(false)
  const [followText, setFollowText] = useState('')
  // Expanded thread messages — React state, not a DOM class, so a poll or parent
  // re-render does not collapse a reply the reader was still reading.
  const [expandedMsgs, setExpandedMsgs] = useState<Set<string>>(() => new Set())

  const closed = q.status !== 'open'
  const blockReason = answerBlockReason(q, draft, labels)
  // While the composer is open the primary IS the follow-up submit. Replaced
  // rather than merely disabled: two primaries with one inert still leaves the
  // reader guessing which one they pressed, and the wrong guess here submits a
  // decision that resumes a ticket.
  const reason = composing ? (followText.trim() ? '' : labels.followFirst) : blockReason

  return (
    <section className={cn('min-h-0 flex-col overflow-hidden', className ?? 'flex')}>
      <div className="min-h-0 grow overflow-y-auto px-6 pt-5 pb-4">
        {/* On a phone this pane REPLACES the question list rather than sitting
            beside it, so it needs a way back. Hidden from `md` up, where both
            are on screen and the control would be meaningless. */}
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            className="text-primary -mx-2 mb-2 flex cursor-pointer items-center gap-1 px-2 py-2 text-[13px] font-[650] md:hidden"
          >
            <span aria-hidden="true">←</span>
            {labels.back ?? 'Back'}
          </button>
        )}
        <div className="text-muted-foreground flex flex-wrap items-center gap-2 font-mono text-[11.5px]">
          <span>{q.ticket}</span>
          <span>·</span>
          <span>
            {labels.askedBy} {q.asked_by}
          </span>
          <span>·</span>
          <span>{fmtAge(q.created_at)}</span>
          {q.mode === 'advisory' && (
            <span className="bg-secondary text-secondary-foreground rounded-[5px] px-1.5">
              {labels.advisory}
            </span>
          )}
        </div>

        {/* Who it is waiting on. A select rather than a typeahead: the list is
            one project's members, and the reader is choosing a colleague, not
            searching a directory. Only while the question is open — reassigning a
            settled one would rewrite who a recorded decision was waiting on, and
            the server refuses it. */}
        {!closed && onAssign && assignable && assignable.length > 0 && (
          <Hint text={labels.assignHint}>
            <label
              className="text-muted-foreground mt-2 flex flex-wrap items-center gap-2 text-[12.5px] font-[650]"
            >
              {labels.assignTo}
              <Picker
                value={q.assignee?.handle ?? ''}
                onValueChange={(v) => onAssign(v || null)}
                className="bg-muted text-foreground border-border max-w-full cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
                options={[
                  { value: '', label: labels.assignNobody },
                  ...assignable.map((p) => ({ value: p.handle, label: p.label })),
                ]}
              />
            </label>
          </Hint>
        )}

        <h1 className="mt-2 mb-0 text-[19px] font-[720] tracking-[-0.02em]">{q.title}</h1>
        {q.body && <Markdown text={q.body} className="mt-3 text-[13.6px]" />}

        {/* The thread: a clear follow-up conversation with the asking agent,
            with the ticket parked until the question is finally answered. */}
        {thread.length > 0 && (
          <div className="mt-5 flex flex-col gap-2">
            {thread.map((m, i) => (
              <div
                key={m.id ?? i}
                className={cn(
                  'rounded-[9px] border px-3 py-2.5 text-[13px]',
                  m.role === 'human'
                    ? 'bg-accent border-ring'
                    : 'bg-card border-border',
                )}
              >
                <div className="text-muted-foreground mb-1 font-mono text-[11px]">
                  {m.author} · {fmtAge(m.created_at)}
                </div>
                <ThreadBody
                  body={m.body}
                  expanded={expandedMsgs.has(m.id ?? String(i))}
                  onToggle={() => {
                    const key = m.id ?? String(i)
                    setExpandedMsgs((cur) => {
                      const next = new Set(cur)
                      if (next.delete(key)) return next
                      next.add(key)
                      return next
                    })
                  }}
                  labels={{ msgMore: labels.msgMore, msgLess: labels.msgLess }}
                />
              </div>
            ))}
            {q.awaiting === 'agent' && (
              <div className="text-muted-foreground text-[12.5px]">
                {labels.waitingAgentPrefix}
                {q.asked_by}
                {labels.waitingAgentSuffix}
              </div>
            )}
          </div>
        )}

        {!closed && (
          <div className="mt-5">
            <AnswerArea
              question={q}
              draft={draft}
              onDraft={onDraft}
              labels={labels}
              disabled={!canAnswer}
            />
            {!canAnswer && (
              <div className="text-muted-foreground mt-2 text-[12.5px]">{labels.readonly}</div>
            )}
          </div>
        )}

        {closed && (
          <div className="text-muted-foreground mt-5 flex items-center gap-3 text-[13px]">
            <span>{labels.closed}</span>
            {/* Only an ANSWERED question can be reopened — the server refuses a
                withdrawn or expired one with `question.not_answered`. Offering
                the button there would be offering an action that always fails. */}
            {q.status === 'answered' && canAnswer && !answerPending && (
              <Button variant="outline" size="sm" onClick={onReopen}>
                {labels.reopen}
              </Button>
            )}
          </div>
        )}
      </div>

      {!closed && (
        <div className="border-t-border-soft bg-card flex flex-col gap-2 border-t px-6 py-3">
          {composing && (
            <div className="flex items-center gap-2">
              <span className="text-muted-foreground shrink-0 text-[12px]">
                {labels.to} <span className="font-mono">{q.asked_by}</span>
              </span>
              <Input
                autoFocus
                placeholder={labels.askFollow}
                value={followText}
                onChange={(e) => setFollowText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && followText.trim()) {
                    e.preventDefault()
                    onFollowup(followText.trim())
                    setFollowText('')
                    setComposing(false)
                  }
                }}
              />
            </div>
          )}

          <div className="min-h-4 text-[12px] text-[color:var(--crit)]" id="qhint">
            {reason}
          </div>

          <div className="flex items-center gap-2">
            <Hint text={reason || undefined}>
              <Button
                aria-disabled={reason ? 'true' : 'false'}
                aria-describedby="qhint"
                className={cn(reason && 'opacity-55')}
                onClick={() => {
                  if (composing) {
                    if (!followText.trim()) return
                    onFollowup(followText.trim())
                    setFollowText('')
                    setComposing(false)
                    return
                  }
                  if (reason) return
                  onSubmit()
                }}
              >
                {composing ? labels.sendFollow : labels.submit}
              </Button>
            </Hint>
            <Hint text={labels.askFollow}>
              <Button
                variant={composing ? 'secondary' : 'outline'}
                aria-label={labels.askFollow}
                onClick={() => setComposing((c) => !c)}
              >
                💬
                {thread.filter((m) => m.role === 'human').length > 0 && (
                  <span className="ml-1 tabular-nums">
                    {thread.filter((m) => m.role === 'human').length}
                  </span>
                )}
              </Button>
            </Hint>
            <span className="grow" />
            <Button variant="ghost" size="sm" onClick={onShare}>
              {labels.share}
            </Button>
            <Button variant="ghost" size="sm" className="text-destructive" onClick={onWithdraw}>
              {labels.withdraw}
            </Button>
          </div>
        </div>
      )}
    </section>
  )
}
