import { fmtAge } from '@/lib/format'
import type { Entry } from '@/lib/initiatives'

export interface SourcesFooterLabels {
  heading: string
  wrote: string
  landed: string
}

export interface SourcesFooterProps {
  /** Cited evidence in reader-facing order; the number shown is the index + 1. */
  sources: Entry[]
  labels: SourcesFooterLabels
  onSelect: (entry: Entry, n: number) => void
}

/**
 * The lineage: every source the document cites, in one list. An initiative that
 * has been fed for months is unreadable as a feed but perfectly readable as a
 * bibliography, and this is the half that makes the prose above checkable.
 */
export function SourcesFooter({ sources, labels, onSelect }: SourcesFooterProps) {
  if (sources.length === 0) return null
  return (
    <div className="border-border mt-8 border-t pt-4">
      <div className="text-muted-foreground mb-2 text-[11px] font-[750] tracking-[0.06em] uppercase">
        {labels.heading}
      </div>
      {sources.map((s, i) => (
        <button
          key={s.id}
          type="button"
          onClick={() => onSelect(s, i + 1)}
          className="border-border-soft hover:bg-accent grid w-full cursor-pointer grid-cols-[26px_minmax(0,1fr)] gap-2.5 border-b px-1 py-1.75 text-left last:border-b-0"
        >
          <span className="text-primary font-mono text-[10.5px] font-extrabold">{i + 1}</span>
          <span className="min-w-0">
            <span className="text-foreground block text-[13px] [overflow-wrap:anywhere]">
              {s.title || s.text || s.filename || s.id}
            </span>
            <span className="text-muted-foreground mt-0.5 flex flex-wrap gap-2.25 font-mono text-[10.5px]">
              <span>{s.kind}</span>
              <span>{s.source}</span>
              {s.origin_at && (
                <span>
                  {labels.wrote} {fmtAge(s.origin_at)}
                </span>
              )}
              <span>
                {labels.landed} {fmtAge(s.created_at)}
              </span>
            </span>
          </span>
        </button>
      ))}
    </div>
  )
}
