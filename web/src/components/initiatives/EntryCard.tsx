import { Markdown } from '@/components/Markdown'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { fmtAge, fmtBytes } from '@/lib/format'
import type { Entry } from '@/lib/initiatives'

export interface EntryCardProps {
  entry: Entry
  labels: { by: string; wrote: string; download: string }
  onDownload: (entry: Entry) => void
}

/**
 * One entry in the collection. Every entry records WHERE IT CAME FROM — source,
 * optional origin time, author — which is what makes the collection weighable
 * later rather than an undifferentiated pile of text.
 */
export function EntryCard({ entry: en, labels, onDownload }: EntryCardProps) {
  return (
    <div className="bg-card border-border mb-2.5 rounded-[10px] border px-4 py-3.5">
      <div className="mb-1.75 flex flex-wrap items-baseline gap-2">
        <Badge
          variant="secondary"
          className="rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase"
        >
          {en.kind}
        </Badge>
        {en.title && <span className="text-[14px] font-[680]">{en.title}</span>}
        <div className="text-muted-foreground ml-auto flex flex-wrap gap-2.5 font-mono text-[11.5px]">
          <span>{en.source}</span>
          {en.origin_at && (
            <span>
              {labels.wrote} {fmtAge(en.origin_at)}
            </span>
          )}
          <span>
            {fmtAge(en.created_at)} · {labels.by} {en.author}
          </span>
        </div>
      </div>

      {en.text && <Markdown text={en.text} className="text-[13.6px]" />}

      {en.source_uri && (
        <div className="mt-1 text-[13.6px]">
          <a
            href={en.source_uri}
            target="_blank"
            rel="noopener noreferrer"
            className="text-[color:var(--accent2)] underline [overflow-wrap:anywhere]"
          >
            {en.source_uri}
          </a>
        </div>
      )}

      {en.has_content && (
        <Button
          variant="outline"
          size="sm"
          className="text-primary mt-2.5 gap-1.75"
          onClick={() => onDownload(en)}
        >
          <span>⬇ {en.filename || labels.download}</span>
          <span className="text-muted-foreground font-mono text-[11.5px] font-semibold">
            {fmtBytes(en.content_bytes)}
          </span>
        </Button>
      )}
    </div>
  )
}
