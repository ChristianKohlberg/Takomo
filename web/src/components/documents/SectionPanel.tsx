// One section of the plan, written out.
//
// A heading, where it stands, what has happened to it, and the prose itself —
// which arrives as `children`, because the prose is a live editor bound to this
// node's own fragment and an editor is not something a presentational component
// can build from props.
//
// The heading is EDITABLE here, and the way it is edited is the whole point.
//
// It used to be read-only, on the argument that a title is one `Y.Text` and a
// caret in two layouts is a fight. That argument holds for a LIVE caret and not
// for this: `EditableText` saves on blur, so a commit here is one `applyText`
// diff — the same shape as the map's own rename, which has always been allowed.
// Two people renaming one section still merge rather than clobber.
//
// Writing a plan and naming its parts is one activity, and sending somebody to
// another view to fix a heading they are looking at is the kind of seam this
// branch exists to remove. "Show it on the map" stays, for when the map is where
// you actually want to be.
//
// Proposals hang off it for the same reason the review button does: an agent
// proposes and a person confirms, and the person confirming is reading the
// section. A section with something waiting SAYS SO in its header, before
// anything is opened — a proposal a reader has to go looking for is one that
// gets accepted by whoever happens to find it, which is not review.
//
// History is the point of this view. `standing` says whether anybody agrees with
// the section as it now reads, and the trace says who did what — the pair is
// what "better diffs" actually comes from, and it is why the review button is
// beside the prose rather than in a panel somewhere else.
import type { ReactNode } from 'react'

import { EditableText } from '@/components/EditableText'
import { Hint } from '@/components/Hint'
import { fmtAge } from '@/lib/format'
import type { TraceEntry, TraceKind } from '@/lib/mindmaps'
import type { Standing } from '@/lib/plan-trace'
import { traceActor } from '@/lib/plan-trace'
import { cn } from '@/lib/utils'

export interface SectionPanelLabels {
  /** Read out by the heading's editor. */
  renameSection: string
  /** A section nobody has given a title yet. */
  untitled: string
  standingConfirmed: string
  standingChanged: string
  standingUnseen: string
  /** The button that records `reviewed`. */
  review: string
  reviewHint: string
  showOnMap: string
  history: string
  hideHistory: string
  historyEmpty: string
  /** `{n}` more entries than are shown. */
  historyMore: string
  /** The toggle that opens what an agent has offered here. */
  proposals: string
  hideProposals: string
  /** The header badge. `{n}` is how many are waiting on a person. */
  pendingBadge: string
  /** What each kind of act is called. */
  kinds: Record<TraceKind, string>
  needWrite: string
}

export interface SectionPanelProps {
  /** `2.1.3`. The shared address of this part of the plan. */
  number: string
  /** 0 for a first-ring node. Heading level, indent and quiet all read it. */
  depth: number
  title: string
  /** Rename the section from here. Absent leaves the heading static. */
  onHeadingEnter?: () => void
  onTitle?: (text: string) => Promise<unknown> | void
  standing: Standing
  /** This section's history, newest first. */
  entries: readonly TraceEntry[]
  historyOpen: boolean
  onToggleHistory: () => void
  onReview: () => void
  onShowOnMap: () => void
  /** How many proposals are waiting on a person here. Drawn in the header, so a
   *  reader finds them without opening anything. */
  pending?: number
  /** How many there are at all, decided ones included — a rejected proposal
   *  stays readable, so the toggle survives its decision. */
  proposalCount?: number
  proposalsOpen?: boolean
  onToggleProposals?: () => void
  /** The review panel itself, when it is open. A slot rather than props,
   *  because what it needs — the section's live editor — is the page's. */
  proposals?: ReactNode
  canWrite: boolean
  /** Highlighted because the rail points at it. */
  onShowTests?: () => void
  testsLabel?: string
  failingTests?: boolean
  onActivate?: () => void
  active?: boolean
  /** How many entries to show before saying there are more. */
  historyLimit?: number
  /** The page's handle on this section's element, for scrolling to it. */
  sectionRef?: (el: HTMLElement | null) => void
  labels: SectionPanelLabels
  children: ReactNode
  className?: string
}

/** Heading level from depth. Stops at h6, which is where HTML stops. */
const HEADINGS = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6'] as const

const headingClass = (depth: number): string =>
  `document-heading document-heading-${Math.min(depth + 1, 6)}`

const STANDING_CLASS: Record<Standing, string> = {
  confirmed: 'border-emerald-300 text-emerald-800 dark:border-emerald-800 dark:text-emerald-300',
  changed: 'border-amber-300 text-amber-900 dark:border-amber-800 dark:text-amber-300',
  unseen: 'border-border-soft text-muted-foreground',
}

export function SectionPanel({
  number,
  depth,
  title,
  onTitle,
  onHeadingEnter,
  standing,
  entries,
  historyOpen,
  onToggleHistory,
  onReview,
  onShowOnMap,
  pending = 0,
  proposalCount = 0,
  proposalsOpen = false,
  onToggleProposals,
  proposals,
  canWrite,
  active = false,
  onActivate,
  onShowTests,
  testsLabel,
  failingTests,
  historyLimit = 6,
  sectionRef,
  labels,
  children,
  className,
}: SectionPanelProps) {
  const Heading = HEADINGS[Math.min(depth, HEADINGS.length - 1)] ?? 'h6'
  const standingLabel: Record<Standing, string> = {
    confirmed: labels.standingConfirmed,
    changed: labels.standingChanged,
    unseen: labels.standingUnseen,
  }
  const shown = entries.slice(0, historyLimit)
  const more = entries.length - shown.length

  return (
    <section
      ref={sectionRef}
      onFocusCapture={onActivate}
      onPointerDown={onActivate}
      className={cn(
        'document-section border-border-soft border-t first:border-t-0',
        active ? 'bg-accent/30' : '',
        className,
      )}

    >
      <div className="mb-2 flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <span className="text-muted-foreground flex-none font-mono text-[11px]">{number}</span>
        {canWrite && onTitle ? (
          <EditableText
            value={title}
            editable
            onCommit={async (next) => {
              await onTitle(next)
            }}
            placeholder={labels.untitled}
            aria-label={labels.renameSection}
            as={Heading}
            onEnter={onHeadingEnter}
            className={cn('min-w-0', headingClass(depth), title ? '' : 'italic opacity-70')}
          />
        ) : (
          <Heading className={cn('min-w-0', headingClass(depth), title ? '' : 'italic opacity-70')}>
            {title || labels.untitled}
          </Heading>
        )}
        <span
          className={cn(
            'flex-none rounded-sm border px-1.5 py-0.5 text-[10.5px] font-[650]',
            STANDING_CLASS[standing],
          )}
        >
          {standingLabel[standing]}
        </span>
        {pending > 0 && (
          <span className="flex-none rounded-sm border border-amber-300 bg-amber-50 px-1.5 py-0.5 text-[10.5px] font-[650] text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200">
            ◆ {labels.pendingBadge.replace('{n}', String(pending))}
          </span>
        )}
      </div>

      <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px]">
        <Hint text={canWrite ? labels.reviewHint : labels.needWrite}>
          <button
            type="button"
            disabled={!canWrite}
            onClick={onReview}
            className="text-muted-foreground hover:text-foreground disabled:opacity-50"
          >
            ✓ {labels.review}
          </button>
        </Hint>
        <button
          type="button"
          onClick={onShowOnMap}
          className="text-muted-foreground hover:text-foreground"
        >
          ⌖ {labels.showOnMap}
        </button>
        {onShowTests && (
          <button
            type="button"
            onClick={onShowTests}
            className={`cursor-pointer text-xs hover:underline ${failingTests ? 'text-destructive' : 'text-muted-foreground'}`}
          >
            {testsLabel}
          </button>
        )}
        {proposalCount > 0 && onToggleProposals && (
          <button
            type="button"
            onClick={onToggleProposals}
            aria-expanded={proposalsOpen}
            className={cn(
              'hover:text-foreground',
              pending > 0 ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground',
            )}
          >
            ◆ {proposalsOpen ? labels.hideProposals : labels.proposals}
          </button>
        )}
        <button
          type="button"
          onClick={onToggleHistory}
          aria-expanded={historyOpen}
          className="text-muted-foreground hover:text-foreground"
        >
          ≡ {historyOpen ? labels.hideHistory : labels.history}
        </button>
      </div>

      {historyOpen && (
        <ul className="text-muted-foreground border-border-soft mb-3 flex flex-col gap-0.5 border-l pl-3 text-[11.5px]">
          {shown.length === 0 && <li>{labels.historyEmpty}</li>}
          {shown.map((entry) => (
            <li key={entry.id} className="flex flex-wrap gap-x-2">
              <span className="text-foreground font-medium">
                {labels.kinds[entry.kind] ?? entry.kind}
              </span>
              <span>{traceActor(entry)}</span>
              <span className="opacity-70">{fmtAge(entry.at)}</span>
              {entry.note && <span className="opacity-80">— {entry.note}</span>}
            </li>
          ))}
          {more > 0 && (
            <li className="opacity-70">{labels.historyMore.replace('{n}', String(more))}</li>
          )}
        </ul>
      )}

      {children}

      {proposalsOpen && proposals}
    </section>
  )
}
