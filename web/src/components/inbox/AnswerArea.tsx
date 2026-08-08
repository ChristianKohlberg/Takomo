// The answer control, per question kind.
//
// Every shape here defaults from the agent's recommendation, so a reader who
// agrees can confirm in one press rather than re-deriving the decision. What
// counts as answerable is `answerBlockReason` in lib/answers.ts — the SAME
// function the submit path calls, so the painted state and the actual refusal
// cannot drift apart.
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Markdown } from '@/components/Markdown'
import { cn } from '@/lib/utils'
import {
  answerType,
  currentMulti,
  currentValue,
  recIsAffirmative,
  type Draft,
} from '@/lib/answers'
import type { Question } from '@/lib/questions'

export interface AnswerAreaLabels {
  yes: string
  no: string
  writeOwn: string
  ownPlaceholder: string
  textPlaceholder: string
  recommends: string
}

export interface AnswerAreaProps {
  question: Question
  draft: Draft | undefined
  onDraft: (patch: Draft) => void
  labels: AnswerAreaLabels
  /** A reader without `human` sees the shape of the decision but cannot make it. */
  disabled?: boolean
}

export function AnswerArea({ question: q, draft, onDraft, labels, disabled }: AnswerAreaProps) {
  const ty = answerType(q)
  const value = currentValue(q, draft)
  const multi = currentMulti(q, draft)
  const rec = recIsAffirmative(q)

  if (ty === 'text') {
    return (
      <div className="flex flex-col gap-2">
        {q.recommended_note && <Recommended note={q.recommended_note} label={labels.recommends} />}
        <Textarea
          className="min-h-24"
          placeholder={labels.textPlaceholder}
          disabled={disabled}
          value={String(value ?? '')}
          onChange={(e) => onDraft({ value: e.target.value })}
        />
      </div>
    )
  }

  if (ty === 'bool') {
    return (
      <div className="flex flex-col gap-2">
        {q.recommended_note && <Recommended note={q.recommended_note} label={labels.recommends} />}
        <div className="flex gap-2">
          {([true, false] as const).map((v) => (
            <Button
              key={String(v)}
              variant={value === v ? 'default' : 'outline'}
              disabled={disabled}
              onClick={() => onDraft({ value: v })}
              className={cn(v === false && value === v && 'bg-destructive border-transparent')}
            >
              {v ? labels.yes : labels.no}
              {/* The agent's recommendation is marked, not hidden: the reader
                  should see what was suggested even when overriding it. */}
              {rec === v && <span className="ml-1.5 opacity-70">★</span>}
            </Button>
          ))}
        </div>
      </div>
    )
  }

  // choose — one option or several, with per-option rationale where the agent
  // supplied it.
  const toggle = (opt: string) => {
    if (ty === 'multi') {
      const next = multi.includes(opt) ? multi.filter((x) => x !== opt) : [...multi, opt]
      onDraft({ multi: next })
    } else {
      onDraft({ value: opt, customOn: false })
    }
  }

  return (
    <div className="flex flex-col gap-2">
      {q.recommended_note && <Recommended note={q.recommended_note} label={labels.recommends} />}
      {q.options.map((opt, i) => {
        const on = ty === 'multi' ? multi.includes(opt) : value === opt && !draft?.customOn
        const recommended =
          ty === 'multi' ? q.recommended_multi?.includes(opt) : q.recommended === opt
        return (
          <button
            key={opt}
            type="button"
            disabled={disabled}
            onClick={() => toggle(opt)}
            aria-pressed={on}
            className={cn(
              'border-border hover:border-ring w-full cursor-pointer rounded-[9px] border px-3 py-2.5 text-left',
              on && 'bg-accent border-ring',
              disabled && 'cursor-not-allowed opacity-60',
            )}
          >
            <div className="flex items-center gap-2 text-[13.5px] font-[650]">
              <span
                className={cn(
                  'border-border inline-block size-3.5 shrink-0 border',
                  ty === 'multi' ? 'rounded-[4px]' : 'rounded-full',
                  on && 'bg-primary border-primary',
                )}
              />
              {opt}
              {recommended && <span className="text-muted-foreground text-[11.5px]">★</span>}
            </div>
            {q.option_notes?.[i] && (
              <div className="text-muted-foreground mt-1 pl-5.5 text-[12.5px]">
                {q.option_notes[i]}
              </div>
            )}
          </button>
        )
      })}

      {ty === 'single' && (
        <div className="flex flex-col gap-1.5">
          <button
            type="button"
            disabled={disabled}
            onClick={() => onDraft({ customOn: !draft?.customOn })}
            className={cn(
              'text-muted-foreground hover:text-primary w-fit cursor-pointer text-[12.5px] font-[650] underline',
              draft?.customOn && 'text-primary',
            )}
          >
            {labels.writeOwn}
          </button>
          {draft?.customOn && (
            <Input
              autoFocus
              disabled={disabled}
              placeholder={labels.ownPlaceholder}
              value={draft.custom ?? ''}
              onChange={(e) => onDraft({ custom: e.target.value })}
            />
          )}
        </div>
      )}
    </div>
  )
}

/** The agent's reasoning for what it suggested — markdown, as it wrote it. */
function Recommended({ note, label }: { note: string; label: string }) {
  return (
    <div className="border-border bg-muted rounded-[9px] border px-3 py-2">
      <div className="text-muted-foreground mb-1 text-[10.5px] font-bold tracking-[0.05em] uppercase">
        {label}
      </div>
      <Markdown text={note} className="text-[12.5px]" />
    </div>
  )
}
