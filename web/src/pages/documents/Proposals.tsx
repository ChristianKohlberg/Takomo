// The review panel: what an agent has offered, and the two buttons.
//
// This is where „der Agent schlägt vor, der Mensch entscheidet" becomes a thing
// you can actually do. Three decisions worth knowing:
//
// **The reason is above the diff.** A reviewer decides on *why*, not on *what* —
// the diff already shows what. So `summary` is the headline and the ops are
// underneath it, which is also why the MCP tool's description tells agents to
// write the reason there.
//
// **`skipped` is shown, not hidden.** When the server drops an op — the block
// was deleted, or it fell outside the scope — the reviewer is looking at a
// proposal that is smaller than the agent intended. Hiding that would let them
// accept something while believing it was the whole change.
//
// **Rejection is recorded, not erased.** A rejected proposal keeps its row with
// who turned it down, because "we considered this and said no" is the thing you
// want three weeks later when it is proposed again.
import { useMemo } from 'react'

import { Button } from '@/components/ui/button'
import type { Op, Proposal } from '@/lib/doc-ops'

export interface ProposalsProps {
  proposals: readonly Proposal[]
  /** The document's current text per block, for the before-side of the diff. */
  textFor: (id: string) => string | null
  canWrite: boolean
  onAccept: (p: Proposal) => void
  onReject: (p: Proposal) => void
  labels: {
    heading: string
    empty: string
    pending: string
    accepted: string
    rejected: string
    accept: string
    reject: string
    by: string
    partial: string
    opReplace: string
    opInsert: string
    opDelete: string
    needWrite: string
  }
}

export function Proposals({
  proposals,
  textFor,
  canWrite,
  onAccept,
  onReject,
  labels,
}: ProposalsProps) {
  // Pending first: a decided proposal is a record, a pending one is work.
  const ordered = useMemo(
    () =>
      [...proposals].sort((a, b) => {
        if (a.status === b.status) return b.created_at - a.created_at
        return a.status === 'pending' ? -1 : b.status === 'pending' ? 1 : 0
      }),
    [proposals],
  )

  if (!ordered.length) {
    return (
      <div className="text-muted-foreground px-1 py-4 text-[12.5px]">
        <p className="mb-1 font-semibold">{labels.heading}</p>
        <p>{labels.empty}</p>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-3 px-1 py-3">
      <p className="text-muted-foreground text-[11.5px] font-bold tracking-wide uppercase">
        {labels.heading}
      </p>
      {ordered.map((p) => (
        <article
          key={p.id}
          className={
            'border-border-soft rounded-md border p-3 text-[13px] ' +
            (p.status === 'pending' ? '' : 'opacity-60')
          }
        >
          <header className="mb-2 flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <span
              className={
                'rounded-sm px-1.5 py-0.5 text-[10.5px] font-bold uppercase ' +
                (p.status === 'pending'
                  ? 'bg-amber-100 text-amber-900 dark:bg-amber-950 dark:text-amber-200'
                  : p.status === 'accepted'
                    ? 'bg-emerald-100 text-emerald-900 dark:bg-emerald-950 dark:text-emerald-200'
                    : 'bg-muted text-muted-foreground')
              }
            >
              {p.status === 'pending'
                ? labels.pending
                : p.status === 'accepted'
                  ? labels.accepted
                  : labels.rejected}
            </span>
            <span className="text-muted-foreground text-[11.5px]">
              {labels.by} {p.author}
            </span>
          </header>

          {/* The reason, first. */}
          {p.summary && <p className="mb-2">{p.summary}</p>}
          {p.instruction && (
            <p className="text-muted-foreground mb-2 text-[12px] italic">“{p.instruction}”</p>
          )}

          {p.skipped?.length > 0 && (
            <p className="mb-2 rounded-sm bg-amber-50 px-2 py-1 text-[11.5px] text-amber-900 dark:bg-amber-950 dark:text-amber-200">
              {labels.partial}
              <br />
              {p.skipped.join('; ')}
            </p>
          )}

          <ul className="flex flex-col gap-2">
            {p.ops.map((op, i) => (
              <li key={`${op.id}-${i}`}>
                <OpDiff op={op} before={textFor(op.id)} labels={labels} />
              </li>
            ))}
          </ul>

          {p.status === 'pending' && (
            <div className="mt-3 flex gap-2">
              <Button
                size="sm"
                disabled={!canWrite}
                title={canWrite ? undefined : labels.needWrite}
                onClick={() => onAccept(p)}
              >
                {labels.accept}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!canWrite}
                title={canWrite ? undefined : labels.needWrite}
                onClick={() => onReject(p)}
              >
                {labels.reject}
              </Button>
            </div>
          )}
        </article>
      ))}
    </div>
  )
}

function OpDiff({
  op,
  before,
  labels,
}: {
  op: Op
  before: string | null
  labels: ProposalsProps['labels']
}) {
  const verb =
    op.op === 'replace'
      ? labels.opReplace
      : op.op === 'insert_after'
        ? labels.opInsert
        : labels.opDelete

  return (
    <div>
      <p className="text-muted-foreground mb-1 font-mono text-[10.5px]">
        {verb} · {op.id}
      </p>
      {/* Whole-block red/green rather than a word-level diff. Two rendered
          paragraphs are hard to compare, but a word-level diff is a bigger piece
          of work than this stage earns — and the block boundary is the unit the
          op is written in anyway. */}
      {op.op !== 'insert_after' && before != null && (
        <p className="mb-1 rounded-sm bg-red-50 px-2 py-1 text-[12.5px] whitespace-pre-wrap text-red-900 line-through dark:bg-red-950/50 dark:text-red-200">
          {before}
        </p>
      )}
      {op.op !== 'delete' && (
        <p className="rounded-sm bg-emerald-50 px-2 py-1 text-[12.5px] whitespace-pre-wrap text-emerald-900 dark:bg-emerald-950/50 dark:text-emerald-200">
          {op.markdown}
        </p>
      )}
    </div>
  )
}
