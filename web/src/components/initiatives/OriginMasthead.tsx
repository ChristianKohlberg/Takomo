import { fmtAge } from '@/lib/format'
import type { Entry } from '@/lib/initiatives'

export interface OriginMastheadProps {
  /** Entries marked `meta.origin`, oldest first. */
  origins: Entry[]
  labels: { heading: string; wrote: string }
}

/**
 * How the idea arrived, quoted at the top of the document.
 *
 * Every pane below is somebody's interpretation. This is the input those
 * interpretations are accountable to, kept verbatim and kept first — so a reader
 * meets the customer's own words before meeting anyone's summary of them.
 */
export function OriginMasthead({ origins, labels }: OriginMastheadProps) {
  if (origins.length === 0) return null
  return (
    <div className="border-border mt-6.5 border-t pt-4">
      <div className="text-muted-foreground mb-2.5 text-[11px] font-[750] tracking-[0.06em] uppercase">
        {labels.heading}
      </div>
      <div className="grid grid-cols-1 gap-3.5 md:grid-cols-2">
        {origins.map((o) => (
          <div key={o.id} className="border-ring min-w-0 border-l-2 py-0.5 pl-3.5">
            <p className="text-foreground m-0 text-[14.5px] leading-[1.45] break-words italic">
              {o.text}
            </p>
            <div className="text-muted-foreground mt-1.5 flex flex-wrap gap-2 font-mono text-[10.5px]">
              <span>{o.source}</span>
              {o.title && <span>{o.title}</span>}
              <span>
                {labels.wrote} {fmtAge(o.origin_at ?? o.created_at)}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
