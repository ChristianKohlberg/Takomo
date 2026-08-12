import { AmendmentCard, type AmendmentCardLabels } from './AmendmentCard'
import { CitationMark } from './CitationMark'
import { MarginThread, type MarginThreadLabels } from './MarginThread'
import { threadsFor, type PaneDoc as PaneDocModel, type Thread } from '@/lib/initiative-doc'
import type { Entry } from '@/lib/initiatives'
import { cn } from '@/lib/utils'

export interface PaneDocLabels extends MarginThreadLabels, AmendmentCardLabels {
  unwritten: string
  unwrittenHint: string
  uncited: string
  citation: string
}

export interface PaneDocProps {
  doc: PaneDocModel
  /** Which source is open in the inspector, so its marks can show as selected. */
  selectedId: string | null
  canWrite: boolean
  busy: boolean
  labels: PaneDocLabels
  onSelectSource: (entry: Entry, n: number) => void
  onDispatch?: (thread: Thread) => void
  onAccept?: () => void
  onReject?: () => void
  /** Rendered in the margin of the FIRST paragraph — the source inspector lives there. */
  inspector?: React.ReactNode
}

/**
 * One pane of the document: prose on the left, margin on the right.
 *
 * Paragraphs and their margin notes are grid ROWS, so a note stays beside the
 * sentence it argues with instead of drifting as the text above it changes
 * length. On a phone the two columns stack — a margin that has to be scrolled
 * past is still better than one that is cut off.
 */
export function PaneDoc({
  doc,
  selectedId,
  canWrite,
  busy,
  labels,
  onSelectSource,
  onDispatch,
  onAccept,
  onReject,
  inspector,
}: PaneDocProps) {
  // A pending amendment is shown even when the pane itself is unwritten — that
  // is the case where accepting it is the whole point.
  const amendment =
    doc.pending && onAccept && onReject ? (
      <AmendmentCard
        amendment={doc.pending}
        canWrite={canWrite}
        busy={busy}
        labels={labels}
        onAccept={onAccept}
        onReject={onReject}
      />
    ) : null

  if (!doc.entry || doc.paragraphs.length === 0) {
    return (
      <>
        {amendment}
        <div className="text-muted-foreground px-2 py-12 text-center">
          <div className="text-foreground mb-1.5 text-[15px] font-[680]">{labels.unwritten}</div>
          <div className="text-[13px]">{labels.unwrittenHint}</div>
        </div>
      </>
    )
  }

  return (
    <>
      {amendment}
      <div className="grid grid-cols-1 gap-x-5 md:grid-cols-[minmax(0,1fr)_minmax(0,296px)]">
        {doc.paragraphs.map((p, i) => {
          const threads = threadsFor(doc, i)
          return (
            <div key={i} className="contents">
              <div className="min-w-0 pt-1 pb-3">
                <p
                  className={cn(
                    'text-foreground m-0 max-w-[64ch] text-[15px] leading-[1.62] [text-wrap:pretty]',
                    // An assertion nobody sourced reads as someone's opinion, and
                    // should look like one.
                    p.uncited &&
                      'decoration-destructive/70 underline decoration-dotted underline-offset-4',
                  )}
                >
                  {p.runs.map((run, j) =>
                    'text' in run ? (
                      <span key={j}>{run.text}</span>
                    ) : (
                      <CitationMark
                        key={j}
                        n={run.cite}
                        label={labels.citation}
                        selected={selectedId === run.entry.id}
                        onSelect={() => onSelectSource(run.entry, run.cite)}
                      />
                    ),
                  )}
                  {p.uncited && (
                    <span className="text-destructive ml-1 align-super font-mono text-[9.5px] font-bold tracking-[0.04em] uppercase">
                      {labels.uncited}
                    </span>
                  )}
                </p>
              </div>

              <div className="mb-3 flex min-w-0 flex-col gap-2 md:mb-0 md:pt-1">
                {i === 0 && inspector}
                {threads.map((t) => (
                  <MarginThread
                    key={t.entry.id}
                    thread={t}
                    canWrite={canWrite}
                    busy={busy}
                    labels={labels}
                    onDispatch={onDispatch}
                  />
                ))}
              </div>
            </div>
          )
        })}
      </div>
    </>
  )
}
