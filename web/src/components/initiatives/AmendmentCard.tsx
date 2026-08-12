import { Button } from '@/components/ui/button'
import { fmtAge } from '@/lib/format'
import type { Amendment } from '@/lib/initiative-doc'

export interface AmendmentCardLabels {
  heading: string
  proposedBy: string
  accept: string
  reject: string
  busy: string
  same: string
  changed: string
  added: string
  removed: string
  was: string
  readOnly: string
}

export interface AmendmentCardProps {
  amendment: Amendment
  canWrite: boolean
  busy: boolean
  labels: AmendmentCardLabels
  onAccept: () => void
  onReject: () => void
}

/**
 * A proposed rewrite of this pane, waiting on a person.
 *
 * The diff is paragraph-level on purpose. An amendment here rewrites whole
 * paragraphs of argument, and a word diff of two paragraphs of prose is noise a
 * reviewer has to decode before they can think — what they need is "these two
 * changed, this one is new", and then to read both versions.
 *
 * Accepting does not edit the pane. It appends the proposed text as a real
 * `view`, so the superseded wording and the decision that replaced it both stay
 * in the log.
 */
export function AmendmentCard({
  amendment,
  canWrite,
  busy,
  labels,
  onAccept,
  onReject,
}: AmendmentCardProps) {
  const { entry, diff } = amendment
  return (
    <div className="border-ring bg-secondary/40 mb-4 rounded-[10px] border">
      <div className="border-border-soft flex flex-wrap items-baseline gap-2 border-b px-3.5 py-2.5">
        <span className="text-secondary-foreground text-[10.5px] font-extrabold tracking-[0.06em] uppercase">
          {labels.heading}
        </span>
        <span className="text-muted-foreground font-mono text-[10.5px]">
          {labels.proposedBy} {entry.source} · {fmtAge(entry.created_at)}
        </span>
      </div>

      <div className="px-3.5 py-3">
        {entry.title && (
          <div className="text-foreground mb-2 text-[13.6px] font-[660]">{entry.title}</div>
        )}
        {diff.map((row, i) => {
          if (row.kind === 'same') {
            return (
              <p
                key={i}
                className="text-muted-foreground m-0 mb-2 line-clamp-2 text-[12.8px] leading-[1.5] break-words"
              >
                {row.text}
              </p>
            )
          }
          const tone =
            row.kind === 'removed'
              ? 'border-destructive/40 bg-destructive/5'
              : 'border-ring bg-secondary'
          return (
            <div key={i} className={`mb-2 rounded-[7px] border px-2.5 py-2 ${tone}`}>
              <div className="text-muted-foreground mb-1 text-[9.5px] font-extrabold tracking-[0.06em] uppercase">
                {row.kind === 'added'
                  ? labels.added
                  : row.kind === 'removed'
                    ? labels.removed
                    : labels.changed}
              </div>
              {row.kind !== 'removed' && (
                <p className="text-foreground m-0 text-[13px] leading-[1.5] break-words">
                  {row.text}
                </p>
              )}
              {row.kind === 'changed' && (
                <p className="text-muted-foreground m-0 mt-1.5 text-[12.4px] leading-[1.45] break-words line-through">
                  {labels.was} {row.was}
                </p>
              )}
              {row.kind === 'removed' && (
                <p className="text-muted-foreground m-0 text-[12.8px] leading-[1.45] break-words line-through">
                  {row.text}
                </p>
              )}
            </div>
          )
        })}
      </div>

      <div className="border-border-soft flex flex-wrap items-center gap-2 border-t px-3.5 py-2.5">
        {canWrite ? (
          <>
            <Button size="sm" disabled={busy} onClick={onAccept}>
              {busy ? labels.busy : labels.accept}
            </Button>
            <Button size="sm" variant="outline" disabled={busy} onClick={onReject}>
              {labels.reject}
            </Button>
          </>
        ) : (
          <span className="text-muted-foreground text-[12.2px]">{labels.readOnly}</span>
        )}
      </div>
    </div>
  )
}
