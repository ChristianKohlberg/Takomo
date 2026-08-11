import { Button } from '@/components/ui/button'
import { fmtAge, fmtBytes } from '@/lib/format'
import type { Entry } from '@/lib/initiatives'

export interface SourceInspectorLabels {
  hint: string
  kind: string
  source: string
  wrote: string
  landed: string
  download: string
  close: string
}

export interface SourceInspectorProps {
  /** The selected source, or null for the standing hint. */
  entry: Entry | null
  n: number | null
  labels: SourceInspectorLabels
  onClose: () => void
  onDownload: (entry: Entry) => void
}

/**
 * Where a sentence came from. `wrote` and `landed` are shown as two rows on
 * purpose — a transcript pasted in five months late has two different, both
 * correct, timestamps, and the gap between them is often the story.
 */
export function SourceInspector({
  entry,
  n,
  labels,
  onClose,
  onDownload,
}: SourceInspectorProps) {
  if (!entry) {
    return (
      <div className="border-border text-muted-foreground rounded-[10px] border border-dashed px-3 py-2.5 text-[12.5px]">
        {labels.hint}
      </div>
    )
  }
  return (
    <div className="bg-secondary border-ring rounded-[10px] border px-3 py-2.75">
      <div className="mb-1.5 flex flex-wrap items-baseline gap-2">
        <span className="bg-primary text-primary-foreground rounded-[3px] px-1.5 font-mono text-[10px] font-extrabold">
          {n}
        </span>
        <span className="text-secondary-foreground text-[10.5px] font-extrabold tracking-[0.06em] uppercase">
          {entry.kind}
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label={labels.close}
          className="text-muted-foreground hover:text-foreground ml-auto cursor-pointer text-[15px] leading-none"
        >
          ×
        </button>
      </div>

      {entry.title && <div className="text-foreground text-[13.5px] font-[660]">{entry.title}</div>}
      {entry.text && (
        <p className="text-muted-foreground mt-1 mb-2 line-clamp-4 text-[13px] break-words">
          {entry.text}
        </p>
      )}

      <Row k={labels.source} v={entry.source} />
      {entry.origin_at && <Row k={labels.wrote} v={fmtAge(entry.origin_at)} />}
      <Row k={labels.landed} v={fmtAge(entry.created_at)} />

      {entry.source_uri && (
        <a
          href={entry.source_uri}
          target="_blank"
          rel="noopener noreferrer"
          className="text-primary mt-1 block font-mono text-[10.5px] [overflow-wrap:anywhere] underline"
        >
          {entry.source_uri}
        </a>
      )}

      {entry.has_content && (
        <Button
          variant="outline"
          size="sm"
          className="text-primary mt-2 gap-1.5"
          onClick={() => onDownload(entry)}
        >
          <span className="min-w-0 truncate">⬇ {entry.filename || labels.download}</span>
          <span className="text-muted-foreground font-mono text-[10.5px] font-semibold">
            {fmtBytes(entry.content_bytes)}
          </span>
        </Button>
      )}
    </div>
  )
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="text-muted-foreground flex justify-between gap-2.5 py-0.25 font-mono text-[10.5px]">
      <span>{k}</span>
      <span className="text-foreground min-w-0 text-right font-[650] [overflow-wrap:anywhere]">
        {v}
      </span>
    </div>
  )
}
