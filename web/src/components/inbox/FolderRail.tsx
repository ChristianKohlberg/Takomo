import { cn } from '@/lib/utils'
import type { Folder } from '@/lib/questions'

export interface FolderRailProps {
  folders: readonly Folder[]
  current: Folder
  /** Per-folder count; Open is the one that matters and is emphasised. */
  counts: Partial<Record<Folder, number>>
  labels: Record<Folder, string> & { heading: string }
  /** Set by the undo queue so a just-answered item tints its destination. */
  landed?: Folder | null
  onSelect: (f: Folder) => void
}

export function FolderRail({
  folders,
  current,
  counts,
  labels,
  landed,
  onSelect,
}: FolderRailProps) {
  return (
    <nav className="bg-card border-r-border-soft min-h-0 overflow-y-auto border-r px-2.5 py-3">
      <div className="text-muted-foreground mb-1.5 px-2 text-[10.5px] font-bold tracking-[0.06em] uppercase">
        {labels.heading}
      </div>
      {folders.map((f) => {
        const n = counts[f] ?? 0
        return (
          <button
            key={f}
            type="button"
            onClick={() => onSelect(f)}
            aria-current={f === current}
            className={cn(
              'hover:bg-muted flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] font-[650]',
              f === current ? 'bg-secondary text-secondary-foreground' : 'text-muted-foreground',
              // A tint that rides along with the movement the reader is already
              // watching: the item leaves Open as this count ticks up.
              landed === f && 'ring-ok ring-1',
            )}
          >
            <span className="grow">{labels[f]}</span>
            {n > 0 && (
              <span
                className={cn(
                  'tabular-nums',
                  f === 'open' ? 'text-primary font-bold' : 'text-muted-foreground',
                )}
              >
                {n}
              </span>
            )}
          </button>
        )
      })}
    </nav>
  )
}
