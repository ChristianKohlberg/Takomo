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
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { spread, worstState, type CaseRow, type CaseState, type Check } from '@/lib/verification'

export interface CheckCardLabels {
  stateFailed: string
  stateStale: string
  stateNever: string
  stateBlocked: string
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
  /** "Verifies <section>" — the part of the plan this check is about. */
  verifiesSection: string
  /** The link from a check to the section it verifies, on the map. */
  openOnMap: string
}

export interface CheckCardProps {
  check: Check
  /**
   * The title of the section this check verifies, resolved from the plan.
   *
   * Resolved rather than stored, so a section renamed on the map is renamed here
   * at once. `undefined` where the check names no section, or names one that has
   * since been pruned — the id is kept either way, because losing the link would
   * be worse than showing one whose section is gone.
   */
  nodeTitle?: string
  /** Show this check's section on the map. Absent when there is no section. */
  onOpenNode?: () => void
  cases: CaseRow[] | undefined
  loadingCases: boolean
  canWrite: boolean
  canApprove: boolean
  labels: CheckCardLabels
  onToggleCases: () => void
  onVerdict: (
    caseId: string,
    verdict: 'pass' | 'fail',
    opts: { note?: string; human?: boolean; environment?: string },
  ) => void
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
    // Louder than `never`: somebody already spent the effort and hit a wall,
    // which usually means an environment to fix.
    case 'blocked':
      return 'bg-accent text-accent-foreground'
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
    case 'blocked':
      return l.stateBlocked
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

/** The controls for one scope: the case as a whole, or one of its environments. */
function VerdictControls({
  caseId,
  environment,
  canWrite,
  canApprove,
  labels,
  onVerdict,
}: {
  caseId: string
  environment?: string
  canWrite: boolean
  canApprove: boolean
  labels: CheckCardLabels
  onVerdict: CheckCardProps['onVerdict']
}) {
  const [failing, setFailing] = useState(false)
  const [note, setNote] = useState('')
  return (
    <>
      {canWrite && (
        <>
          <Button
            variant="outline"
            size="sm"
            onClick={() => onVerdict(caseId, 'pass', { environment })}
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
        <Button size="sm" onClick={() => onVerdict(caseId, 'pass', { human: true, environment })}>
          {labels.approve}
        </Button>
      )}
      {failing && (
        <div className="flex w-full flex-col gap-2 md:flex-row">
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
              onVerdict(caseId, 'fail', { note: note.trim(), environment })
              setFailing(false)
              setNote('')
            }}
          >
            {labels.markFail}
          </Button>
        </div>
      )}
    </>
  )
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
  // With declared environments the controls move DOWN to each environment: a
  // single Pass button there would have to pick one silently, which is the
  // failure the server refuses at the door.
  const perEnv = row.environments.length > 0
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
        {!perEnv && (
          <VerdictControls
            caseId={row.id}
            canWrite={canWrite}
            canApprove={canApprove}
            labels={labels}
            onVerdict={onVerdict}
          />
        )}
      </div>
      {perEnv && (
        <div className="flex flex-col gap-1.5 pl-1">
          {row.environments.map((e) => (
            <div key={e.environment} className="flex min-w-0 flex-wrap items-center gap-2">
              <Badge className={stateTone(e.state)}>{stateWord(e.state, labels)}</Badge>
              <span className="text-muted-foreground min-w-0 truncate text-[12px] font-[620]">
                {e.slug}
              </span>
              <span className="grow" />
              <VerdictControls
                caseId={row.id}
                environment={e.environment}
                canWrite={canWrite}
                canApprove={canApprove}
                labels={labels}
                onVerdict={onVerdict}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

export function CheckCard({
  check,
  nodeTitle,
  onOpenNode,
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
    <Card size="sm" className="gap-2.5 px-(--card-spacing)">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <Badge className={stateTone(worst)}>{stateWord(worst, labels)}</Badge>
        <span className="text-foreground min-w-0 grow truncate text-[13.5px] font-[650]">
          {check.title}
        </span>
        <Badge className="bg-muted text-muted-foreground">{check.severity}</Badge>
        <Badge className="bg-muted text-muted-foreground">{check.layer}</Badge>
      </div>

      {/* The part of the plan this verifies. A check nobody filed under a
          section shows nothing rather than an empty label. */}
      {check.node && (
        <div className="text-muted-foreground flex min-w-0 flex-wrap items-center gap-1.5 text-[12px]">
          <span className="shrink-0">{labels.verifiesSection}</span>
          {onOpenNode ? (
            <button
              type="button"
              onClick={onOpenNode}
              className="text-foreground min-w-0 cursor-pointer truncate underline underline-offset-2"
              title={labels.openOnMap}
            >
              {nodeTitle ?? check.node}
            </button>
          ) : (
            <span className="text-foreground min-w-0 truncate">{nodeTitle ?? check.node}</span>
          )}
        </div>
      )}

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

      {/* Where it must pass, and how each one stands. On the face rather than
          behind the disclosure: "fine on staging, untouched on production" is
          the finding, and a single rolled-up word hides exactly that. */}
      {check.environment_cases.length > 0 && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px]">
          {check.environment_cases.map((e) => (
            <span key={e.environment} className="flex items-center gap-1.5">
              <Badge className={stateTone(worstState(e.cases))}>{e.slug}</Badge>
              <span className="text-muted-foreground">
                {stateWord(worstState(e.cases), labels)}
              </span>
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
    </Card>
  )
}
