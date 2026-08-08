// The `#s=` page: a read-only board someone was handed a link to.
//
// A `tks_` token reaches `/v1/shares/self*` and nothing else, so this renders
// from a different surface than the signed-in board — same columns, no actions,
// no token entry. There is deliberately no way to escalate from here into the
// real board: the share is the whole grant.
import { useEffect, useMemo, useState } from 'react'
import { Logo } from '@/components/Logo'
import { Column } from '@/components/board/Column'
import { fmtWhen } from '@/lib/cadence'
import { shareSelf, shareTickets, type ShareSelf, type ShareTicket } from '@/lib/grants'
import type { Ticket } from '@/lib/board'
import type { Locale } from '@/lib/i18n'

export interface SharePageProps {
  token: string
  lang: Locale
  labels: {
    readOnly: string
    validUntil: string
    expired: string
    showMore: string
    blocked: string
    empty: string
    fromSchedule: string
    notFulfilled: string
  }
}

export function SharePage({ token, lang, labels }: SharePageProps) {
  const [meta, setMeta] = useState<ShareSelf | null>(null)
  const [tickets, setTickets] = useState<ShareTicket[]>([])
  const [error, setError] = useState('')

  useEffect(() => {
    let cancelled = false
    Promise.all([shareSelf(token), shareTickets(token)])
      .then(([m, ts]) => {
        if (cancelled) return
        setMeta(m)
        setTickets(ts)
      })
      .catch((e: { status?: number; message?: string }) => {
        if (cancelled) return
        setError(e?.status === 401 || e?.status === 403 ? labels.expired : (e?.message ?? ''))
      })
    return () => {
      cancelled = true
    }
  }, [token, labels])

  // Columns come from the share's own workflow — the same states the project
  // uses, served alongside the tickets so this page needs no second credential.
  const states = useMemo(() => meta?.workflow?.states?.map((s) => s.id) ?? [], [meta])
  const byState = useMemo(() => {
    const m = new Map<string, ShareTicket[]>()
    for (const s of states) m.set(s, [])
    for (const t of tickets) {
      if (!m.has(t.state)) m.set(t.state, [])
      m.get(t.state)!.push(t)
    }
    return m
  }, [tickets, states])

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <header className="bg-card border-b-border-soft flex min-h-[58px] flex-none flex-wrap items-center gap-3 border-b px-5 py-2.5">
        <div className="flex items-center gap-2.5 text-[color:var(--accent2)]">
          <Logo />
          <span className="text-foreground text-base font-[750] tracking-[-0.02em]">takomo</span>
        </div>
        <span className="bg-secondary text-secondary-foreground rounded-[6px] px-2 py-0.5 text-[11.5px] font-[680]">
          {labels.readOnly}
        </span>
        {meta?.project && (
          <span className="text-muted-foreground font-mono text-[12px]">{meta.project}</span>
        )}
        <span className="grow" />
        {meta?.expires_at && (
          <span className="text-muted-foreground font-mono text-[11.5px]">
            {labels.validUntil} {fmtWhen(meta.expires_at, lang)}
          </span>
        )}
      </header>

      <main className="min-h-0 flex-1 overflow-x-auto p-4">
        {error ? (
          <div className="text-destructive px-2 py-10 text-center text-[14px]">{error}</div>
        ) : tickets.length === 0 ? (
          <div className="text-muted-foreground px-2 py-10 text-center text-[13.5px]">
            {labels.empty}
          </div>
        ) : (
          <div className="flex h-full min-h-0 gap-3">
            {[...byState.entries()].map(([state, ts]) => (
              <Column
                key={state}
                state={state}
                tickets={ts as Ticket[]}
                labels={{ showMore: labels.showMore, blocked: labels.blocked, fromSchedule: labels.fromSchedule, notFulfilled: labels.notFulfilled }}
                onOpen={() => {
                  /* read-only: a share has no detail route to open */
                }}
              />
            ))}
          </div>
        )}
      </main>
    </div>
  )
}
