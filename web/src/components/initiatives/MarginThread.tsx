import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { fmtAge } from '@/lib/format'
import { cn } from '@/lib/utils'
import type { Thread } from '@/lib/initiative-doc'

export interface MarginThreadLabels {
  open: string
  running: string
  resolved: string
  dispatch: string
  dispatching: string
  ticket: string
}

export interface MarginThreadProps {
  thread: Thread
  canWrite: boolean
  busy: boolean
  labels: MarginThreadLabels
  /** Files a ticket for this note and marks it running. Omit to render read-only. */
  onDispatch?: (thread: Thread) => void
}

/**
 * A note in the margin, anchored to the paragraph that provoked it.
 *
 * This is where an action belongs. A global "what should we research?" bar asks
 * the question in the abstract; a margin note asks it about a specific sentence
 * somebody just read and did not believe — and the answer comes back to the same
 * place.
 *
 * Dispatching does not mutate this entry. It appends a new `thread` that
 * supersedes it, so the note, its ticket and the moment somebody acted on it are
 * three readable facts rather than one overwritten row.
 */
export function MarginThread({
  thread,
  canWrite,
  busy,
  labels,
  onDispatch,
}: MarginThreadProps) {
  const { entry, state, ticket } = thread
  const label =
    state === 'running' ? labels.running : state === 'resolved' ? labels.resolved : labels.open
  return (
    <div
      className={cn(
        'bg-card border-border rounded-[10px] border px-3 py-2.5',
        state === 'running' && 'border-ring',
        state === 'resolved' && 'opacity-70',
      )}
    >
      <div className="mb-1.5 flex flex-wrap items-baseline gap-2">
        <span className="text-muted-foreground font-mono text-[10.5px]">{entry.source}</span>
        <span className="text-muted-foreground font-mono text-[10.5px]">
          {fmtAge(entry.created_at)}
        </span>
        <Badge
          variant="secondary"
          className="ml-auto rounded-[5px] px-1.5 py-0 text-[9.5px] font-[750] tracking-[0.05em] uppercase"
        >
          {label}
        </Badge>
      </div>

      {entry.title && (
        <div className="text-foreground mb-0.5 text-[12.8px] font-[650]">{entry.title}</div>
      )}
      {entry.text && (
        <p className="text-muted-foreground m-0 text-[12.8px] leading-[1.45] break-words">
          {entry.text}
        </p>
      )}

      {ticket && (
        <div className="text-muted-foreground mt-2 font-mono text-[10.5px]">
          {labels.ticket} {ticket}
        </div>
      )}

      {state === 'open' && canWrite && onDispatch && (
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          className="text-primary mt-2.5"
          onClick={() => onDispatch(thread)}
        >
          {busy ? labels.dispatching : labels.dispatch}
        </Button>
      )}
    </div>
  )
}
