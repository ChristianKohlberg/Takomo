import { fmtAge, fmtBytes } from '@/lib/format'
import type { Rollup } from '@/lib/initiatives'

export interface RollupStripProps {
  rollup: Rollup | undefined
  labels: {
    entries: string
    attachments: string
    chars: string
    size: string
    last: string
  }
}

/** What has accumulated on an initiative: the case for it being worth keeping. */
export function RollupStrip({ rollup, labels }: RollupStripProps) {
  const r = rollup ?? {}
  const cells: [string, string][] = [
    [labels.entries, String(r.entries ?? 0)],
    [labels.attachments, String(r.attachments ?? 0)],
    [labels.chars, (r.chars ?? 0).toLocaleString()],
    [labels.size, fmtBytes(r.bytes)],
    [labels.last, fmtAge(r.last_entry_at)],
  ]
  return (
    <div className="bg-card border-border mt-4.5 mb-1.5 flex flex-wrap overflow-hidden rounded-[10px] border">
      {cells.map(([k, v], idx) => (
        <div
          key={k}
          className={
            'flex-[1_1_110px] px-3.5 py-2.75' +
            (idx < cells.length - 1 ? ' border-r-border-soft border-r' : '')
          }
        >
          <div className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {k}
          </div>
          <div className="mt-0.5 text-[17px] font-[720] tabular-nums">{v}</div>
        </div>
      ))}
    </div>
  )
}
