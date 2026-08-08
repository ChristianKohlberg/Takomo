// One snack per pending answer, each with its own countdown and its own Undo.
//
// Stacked rather than replaced: working through the inbox quickly leaves several
// running side by side, and collapsing them into one would make it impossible to
// undo the second-to-last decision.
import { Button } from '@/components/ui/button'
import { secondsLeft, type Pending } from '@/lib/undo-queue'

export interface UndoSnackbarProps {
  pending: Pending[]
  /** Passed in rather than read from the clock, so the row is a pure render. */
  now: number
  labels: { undo: string; seconds: string }
  onUndo: (qid: string) => void
}

export function UndoSnackbar({ pending, now, labels, onUndo }: UndoSnackbarProps) {
  if (!pending.length) return null
  return (
    <div className="fixed bottom-4 left-1/2 z-90 flex -translate-x-1/2 flex-col gap-2">
      {pending.map((p) => (
        <div
          key={p.qid}
          role="status"
          className="bg-foreground text-background flex items-center gap-3 rounded-[10px] px-3.5 py-2.5 text-[13px] shadow-[0_18px_40px_-18px_rgba(0,0,0,.5)]"
        >
          <div className="min-w-0">
            <div className="truncate font-[650]">{p.decision}</div>
            <div className="truncate opacity-70">{p.detail}</div>
          </div>
          <span className="tabular-nums opacity-70">
            {secondsLeft(p, now)}
            {labels.seconds}
          </span>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onUndo(p.qid)}
            className="shrink-0"
          >
            {labels.undo}
          </Button>
        </div>
      ))}
    </div>
  )
}
