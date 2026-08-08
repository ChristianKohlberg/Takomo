// The board's own inbox: read AND answer open questions without leaving the
// board.
//
// This is not a shortcut to /inbox — it is the same decisions, reachable from
// where you noticed them. A question whose turn is a human's gets the emphatic
// treatment; one already bounced back to the agent gets a quieter one, because
// there is nothing for the reader to do on it yet and pretending otherwise
// wastes the most valuable thing on this surface, which is their attention.
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Markdown } from '@/components/Markdown'
import { AnswerArea } from '@/components/inbox/AnswerArea'
import { cn } from '@/lib/utils'
import { answerBlockReason, answerPayloadFor, type Draft } from '@/lib/answers'
import type { Question } from '@/lib/questions'

export interface InboxDrawerLabels {
  title: string
  empty: string
  emptySub: string
  blocking: string
  advisory: string
  inConversation: string
  awaiting: string
  awaitingSub: string
  recommends: string
  notePlaceholder: string
  send: string
  cantAnswer: string
  close: string
  /** `approve` questions are approved or rejected; the rest are yes/no. */
  approve: string
  reject: string
  yes: string
  no: string
  writeOwn: string
  ownPlaceholder: string
  textPlaceholder: string
  typeFirst: string
  sendFirst: string
}

export interface InboxDrawerProps {
  open: boolean
  questions: Question[]
  canAnswer: boolean
  labels: InboxDrawerLabels
  onClose: () => void
  onAnswer: (q: Question, value: unknown, note: string) => Promise<unknown>
}

const URGENCY: Record<string, string> = {
  critical: 'text-crit',
  high: 'text-high',
  normal: 'text-normal',
  low: 'text-low',
}

export function InboxDrawer({
  open,
  questions,
  canAnswer,
  labels,
  onClose,
  onAnswer,
}: InboxDrawerProps) {
  const [drafts, setDrafts] = useState<Record<string, Draft>>({})
  const [notes, setNotes] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState<string | null>(null)
  const [errors, setErrors] = useState<Record<string, string>>({})

  if (!open) return null

  return (
    <>
      <div className="bg-[color:var(--overlay)] fixed inset-0 z-20" onClick={onClose} />
      <aside className="bg-card border-border fixed top-0 right-0 z-21 h-full w-[min(520px,100%)] overflow-y-auto border-l shadow-[-24px_0_60px_-30px_rgba(20,40,55,.5)]">
        <header className="bg-card border-b-border-soft sticky top-0 flex items-center gap-2 border-b px-6 py-4">
          <h2 className="m-0 flex-1 text-[18px] font-[720] tracking-[-0.02em]">{labels.title}</h2>
          <button
            type="button"
            aria-label={labels.close}
            onClick={onClose}
            className="text-muted-foreground cursor-pointer px-1.5 text-[20px] leading-none"
          >
            ×
          </button>
        </header>

        <div className="flex flex-col gap-3 px-6 py-4">
          {questions.length === 0 ? (
            <div className="text-muted-foreground py-10 text-center">
              <div className="text-foreground mb-1.5 text-[15px] font-[680]">{labels.empty}</div>
              <div className="text-[13px]">{labels.emptySub}</div>
            </div>
          ) : (
            questions.map((q) => {
              const conv = q.awaiting === 'agent'
              const blocking = q.mode !== 'advisory'
              const draft = drafts[q.id]
              const reason = answerBlockReason(q, draft, labels)
              return (
                <div
                  key={q.id}
                  className={cn(
                    'rounded-[10px] border px-3.5 py-3',
                    conv
                      ? 'border-border bg-muted'
                      : blocking
                        ? 'border-ring bg-accent'
                        : 'border-border bg-card',
                  )}
                >
                  <div className="flex flex-wrap items-center gap-2 text-[11px] font-[680]">
                    <span className={URGENCY[q.urgency ?? 'normal'] ?? 'text-normal'}>
                      {q.urgency ?? 'normal'}
                    </span>
                    <span className="text-muted-foreground">
                      {blocking ? labels.blocking : labels.advisory}
                    </span>
                  </div>

                  <div className="mt-1 text-[13.5px] font-[680]">{q.title}</div>
                  <div className="text-muted-foreground mt-0.5 font-mono text-[11px]">
                    {[q.asked_by ?? '—', q.kind, q.ticket].join(' · ')}
                  </div>

                  {conv && (
                    <div className="text-muted-foreground mt-1.5 text-[12px]">
                      ⤺ {labels.inConversation} · {labels.awaiting}
                      {q.asked_by ?? 'agent'}
                      {labels.awaitingSub}
                    </div>
                  )}

                  {q.body && <Markdown text={q.body} className="mt-2 text-[13px]" />}

                  {/* Answering is only offered where it is the reader's turn: a
                      question bounced back to the agent has nothing to answer. */}
                  {!conv && canAnswer && (
                    <div className="mt-3 flex flex-col gap-2">
                      <AnswerArea
                        question={q}
                        draft={draft}
                        onDraft={(patch) =>
                          setDrafts((d) => ({ ...d, [q.id]: { ...d[q.id], ...patch } }))
                        }
                        // Per question, not per drawer: the list mixes kinds, and
                        // "Yes" on an approve question understates what is being
                        // authorized.
                        labels={{
                          ...labels,
                          yes: q.kind === 'approve' ? labels.approve : labels.yes,
                          no: q.kind === 'approve' ? labels.reject : labels.no,
                        }}
                      />
                      <Input
                        placeholder={labels.notePlaceholder}
                        value={notes[q.id] ?? ''}
                        onChange={(e) => setNotes((n) => ({ ...n, [q.id]: e.target.value }))}
                      />
                      {(errors[q.id] || reason) && (
                        <div className="text-destructive text-[12px]">
                          {errors[q.id] || reason}
                        </div>
                      )}
                      <Button
                        size="sm"
                        className="w-fit"
                        disabled={busy === q.id}
                        aria-disabled={reason ? 'true' : 'false'}
                        onClick={() => {
                          if (reason) return
                          setBusy(q.id)
                          setErrors((e) => ({ ...e, [q.id]: '' }))
                          onAnswer(q, answerPayloadFor(q, draft).value, notes[q.id] ?? '')
                            .catch((e: Error) =>
                              setErrors((cur) => ({ ...cur, [q.id]: e.message })),
                            )
                            .finally(() => setBusy(null))
                        }}
                      >
                        {labels.send}
                      </Button>
                    </div>
                  )}

                  {!conv && !canAnswer && (
                    <div className="text-muted-foreground mt-2 text-[12px]">
                      {labels.cantAnswer}
                    </div>
                  )}
                </div>
              )
            })
          )}
        </div>
      </aside>
    </>
  )
}
