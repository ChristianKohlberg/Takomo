// The `#a=` page: one question, for someone who has nothing else.
//
// A `tka_` grant shows exactly one question and lets its holder answer it once.
// The reader is typically an outside expert with NO other context — which is why
// the question body is rendered as markdown here like on every other surface.
// It was once the one place that missed it, so an outside expert saw `## Frage`
// and `| Option | Risiko |` as literal source while every internal reader saw
// them rendered. That reader can least afford it.
import { useEffect, useState } from 'react'
import { Logo } from '@/components/Logo'
import { Markdown } from '@/components/Markdown'
import { AnswerArea } from '@/components/inbox/AnswerArea'
import { Button } from '@/components/ui/button'
import { answerPayloadFor, answerBlockReason, type Draft } from '@/lib/answers'
import { answerGrantSelf, submitGrantAnswer, type AnswerGrant } from '@/lib/grants'
import { fmtWhen } from '@/lib/cadence'
import type { Locale } from '@/lib/i18n'

export interface AnswerGrantLabels {
  yes: string
  no: string
  writeOwn: string
  ownPlaceholder: string
  textPlaceholder: string
  recommends: string
  submit: string
  typeFirst: string
  sendFirst: string
  ticketCtx: string
  validUntil: string
  thanks: string
  spent: string
  expired: string
}

export interface AnswerGrantPageProps {
  token: string
  lang: Locale
  labels: AnswerGrantLabels
}

export function AnswerGrantPage({ token, lang, labels }: AnswerGrantPageProps) {
  const [grant, setGrant] = useState<AnswerGrant | null>(null)
  const [error, setError] = useState('')
  const [draft, setDraft] = useState<Draft>({})
  const [done, setDone] = useState(false)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    answerGrantSelf(token)
      .then((g) => !cancelled && setGrant(g))
      .catch((e: { status?: number; message?: string }) => {
        if (cancelled) return
        // A spent or expired link is the common case, not an error to debug —
        // say which, in plain words.
        setError(e?.status === 410 ? labels.spent : (e?.message ?? labels.expired))
      })
    return () => {
      cancelled = true
    }
  }, [token, labels])

  if (done) {
    return (
      <Shell>
        <p className="text-[15px] font-[650]">{labels.thanks}</p>
      </Shell>
    )
  }
  if (error) {
    return (
      <Shell>
        <p className="text-destructive text-[14px]">{error}</p>
      </Shell>
    )
  }
  if (!grant) return <Shell />

  const q = grant.question
  const reason = answerBlockReason(q, draft, labels)

  return (
    <Shell>
      <div className="text-muted-foreground font-mono text-[11.5px]">
        {q.ticket} · {labels.validUntil} {fmtWhen(grant.expires_at, lang)}
      </div>
      <h1 className="mt-2 mb-0 text-[20px] font-[730] tracking-[-0.02em]">{q.title}</h1>
      {q.body && <Markdown text={q.body} className="mt-3 text-[13.6px]" />}

      {grant.ticket?.title && (
        <div className="border-border bg-muted mt-4 rounded-[9px] border px-3 py-2.5">
          <div className="text-muted-foreground mb-1 text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {labels.ticketCtx}
          </div>
          <div className="text-[13px] font-[650]">{grant.ticket.title}</div>
          {grant.ticket.body && (
            <Markdown text={grant.ticket.body} className="mt-1 text-[12.5px]" />
          )}
        </div>
      )}

      <div className="mt-5">
        <AnswerArea
          question={q}
          draft={draft}
          onDraft={(patch) => setDraft((d) => ({ ...d, ...patch }))}
          labels={labels}
        />
      </div>

      <div className="text-destructive mt-3 min-h-4 text-[12px]">{reason}</div>
      <Button
        disabled={busy}
        aria-disabled={reason ? 'true' : 'false'}
        className={reason ? 'opacity-55' : undefined}
        onClick={() => {
          if (reason) return
          setBusy(true)
          submitGrantAnswer(token, answerPayloadFor(q, draft).value)
            .then(() => setDone(true))
            .catch((e: { status?: number; message?: string }) => {
              // A rejected answer rolls the spend back server-side, so the
              // reader can genuinely try again — do not lock the form.
              setError(e?.status === 410 ? labels.spent : (e?.message ?? labels.expired))
            })
            .finally(() => setBusy(false))
        }}
      >
        {labels.submit}
      </Button>
    </Shell>
  )
}

function Shell({ children }: { children?: React.ReactNode }) {
  return (
    <div className="mx-auto max-w-2xl px-6 py-[8vh]">
      <div className="mb-6 flex items-center gap-2.5 text-[color:var(--accent2)]">
        <Logo />
        <span className="text-foreground text-base font-[750] tracking-[-0.02em]">takomo</span>
      </div>
      {children}
    </div>
  )
}
