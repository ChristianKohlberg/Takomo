// One check, with its cases behind a disclosure.
//
// The collapsed row carries the SPREAD rather than a percentage: "8 verified ·
// 3 stale · 1 never" is the finding, and averaging it into 67% is how a
// regression hides. `never` and `stale` stay separate for the same reason —
// one is work nobody has done once, the other is work that was done and then
// invalidated by a merge, and merging them shrinks the gap this page exists to
// show.
import { useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { spread, worstState, type CaseRow, type CaseState, type Check } from '@/lib/verification'

export interface CheckCardLabels {
  stateFailed: string
  stateStale: string
  stateNever: string
  stateUnreachable: string
  stateVerified: string
  stateApproved: string
  stateNone: string
  orphanGlobs: string
  showCases: string
  hideCases: string
  noCases: string
  approve: string
  markPass: string
  markFail: string
  notePlaceholder: string
  archiveCheck: string
}

export interface CheckCardProps {
  check: Check
  cases: CaseRow[] | undefined
  loadingCases: boolean
  canWrite: boolean
  canApprove: boolean
  labels: CheckCardLabels
  onToggleCases: () => void
  onVerdict: (caseId: string, verdict: 'pass' | 'fail', opts: { note?: string; human?: boolean }) => void
  onArchive: () => void
}

/** Bad news is loud; good news is quiet. */
export function stateTone(state: CaseState | 'none'): string {
  switch (state) {
    case 'failed':
      return 'bg-nfbg text-nf border-nfbd'
    case 'stale':
      return 'bg-accent text-accent-foreground'
    case 'never':
      return 'bg-muted text-muted-foreground'
    case 'approved':
    case 'verified':
      return 'bg-ok-bg text-ok'
    default:
      return 'bg-muted text-muted-foreground'
  }
}

export function stateWord(state: CaseState | 'none', l: CheckCardLabels): string {
  switch (state) {
    case 'failed':
      return l.stateFailed
    case 'stale':
      return l.stateStale
    case 'never':
      return l.stateNever
    case 'unreachable':
      return l.stateUnreachable
    case 'verified':
      return l.stateVerified
    case 'approved':
      return l.stateApproved
    default:
      return l.stateNone
  }
}

function CaseLine({
  row,
  canWrite,
  canApprove,
  labels,
  onVerdict,
}: {
  row: CaseRow
  canWrite: boolean
  canApprove: boolean
  labels: CheckCardLabels
  onVerdict: CheckCardProps['onVerdict']
}) {
  const [failing, setFailing] = useState(false)
  const [note, setNote] = useState('')

  return (
    <div className="border-border-soft flex flex-col gap-2 border-t py-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <Badge className={stateTone(row.state as CaseState)}>
          {stateWord(row.state as CaseState, labels)}
        </Badge>
        <span className="text-foreground min-w-0 truncate font-mono text-[12px]">{row.key}</span>
        {row.label && row.label !== row.key && (
          <span className="text-muted-foreground min-w-0 truncate text-[12px]">{row.label}</span>
        )}
        <span className="grow" />
        {canWrite && (
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => onVerdict(row.id, 'pass', {})}
            >
              {labels.markPass}
            </Button>
            <Button variant="outline" size="sm" onClick={() => setFailing((v) => !v)}>
              {labels.markFail}
            </Button>
          </>
        )}
        {/* Approving asserts a PERSON checked it — the same line ask-a-human
            draws — so it is a separate control from recording an observation. */}
        {canApprove && (
          <Button size="sm" onClick={() => onVerdict(row.id, 'pass', { human: true })}>
            {labels.approve}
          </Button>
        )}
      </div>
      {failing && (
        <div className="flex flex-col gap-2 md:flex-row">
          <Input
            value={note}
            placeholder={labels.notePlaceholder}
            onChange={(e) => setNote(e.target.value)}
          />
          <Button
            variant="outline"
            size="sm"
            disabled={!note.trim()}
            onClick={() => {
              onVerdict(row.id, 'fail', { note: note.trim() })
              setFailing(false)
              setNote('')
            }}
          >
            {labels.markFail}
          </Button>
        </div>
      )}
    </div>
  )
}

export function CheckCard({
  check,
  cases,
  loadingCases,
  canWrite,
  canApprove,
  labels,
  onToggleCases,
  onVerdict,
  onArchive,
}: CheckCardProps) {
  const worst = worstState(check.cases)
  const parts = spread(check.cases)
  const open = cases !== undefined

  return (
    <div className="bg-card border-border-soft flex flex-col gap-2.5 rounded-[10px] border p-3.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <Badge className={stateTone(worst)}>{stateWord(worst, labels)}</Badge>
        <span className="text-foreground min-w-0 grow truncate text-[13.5px] font-[650]">
          {check.title}
        </span>
        <Badge className="bg-muted text-muted-foreground">{check.severity}</Badge>
        <Badge className="bg-muted text-muted-foreground">{check.layer}</Badge>
      </div>

      {parts.length > 0 && (
        <div className="text-muted-foreground flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px]">
          {parts.map((p, i) => (
            <span key={p.state} className={cn(p.state === 'failed' && 'text-nf font-[650]')}>
              {i > 0 && <span className="mr-2 opacity-40">·</span>}
              {p.n} {stateWord(p.state, labels)}
            </span>
          ))}
        </div>
      )}

      {check.precondition && (
        <p className="text-muted-foreground text-[12.5px]">{check.precondition}</p>
      )}

      {/* The worst failure mode this feature has: a glob that matches nothing
          still reads as covered, inflating confidence exactly where it is
          unwarranted. So it is stated, not tucked away. */}
      {check.orphan_globs.length > 0 && (
        <p className="text-nf text-[12px] font-[620]">
          {labels.orphanGlobs}: {check.orphan_globs.join(', ')}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" onClick={onToggleCases}>
          {open ? labels.hideCases : `${labels.showCases} (${check.cases.total})`}
        </Button>
        {canWrite && (
          <Button variant="outline" size="sm" onClick={onArchive}>
            {labels.archiveCheck}
          </Button>
        )}
      </div>

      {open && (
        <div className="flex flex-col">
          {loadingCases ? null : cases.length === 0 ? (
            <p className="text-muted-foreground border-border-soft border-t pt-2 text-[12.5px]">
              {labels.noCases}
            </p>
          ) : (
            cases.map((row) => (
              <CaseLine
                key={row.id}
                row={row}
                canWrite={canWrite}
                canApprove={canApprove}
                labels={labels}
                onVerdict={onVerdict}
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}
