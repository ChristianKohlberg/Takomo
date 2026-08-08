import { StatusBadge } from './StatusBadge'
import { cn } from '@/lib/utils'
import { fmtAge, fmtBytes } from '@/lib/format'
import type { Initiative } from '@/lib/initiatives'

export interface InitiativeRowProps {
  initiative: Initiative
  selected: boolean
  statusLabel: string
  /** Lower-cased noun for the entry count, e.g. "entries" / "einträge". */
  entriesWord: string
  onSelect: (id: string) => void
}

export function InitiativeRow({
  initiative: i,
  selected,
  statusLabel,
  entriesWord,
  onSelect,
}: InitiativeRowProps) {
  const r = i.rollup ?? {}
  return (
    <div
      role="button"
      tabIndex={0}
      aria-current={selected || undefined}
      onClick={() => onSelect(i.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelect(i.id)
        }
      }}
      className={cn(
        'border-b-border-soft hover:bg-muted flex cursor-pointer flex-col gap-1.25 border-b px-4.5 py-3.25',
        selected && 'bg-accent shadow-[inset_3px_0_0_var(--accent2)]',
      )}
    >
      <div className="flex items-baseline gap-2 text-[13.8px] font-[680] tracking-[-0.01em]">
        <span className="min-w-0 grow">{i.title}</span>
        <StatusBadge status={i.status} label={statusLabel} />
      </div>
      {i.summary && (
        <div className="text-muted-foreground overflow-hidden text-[12.5px] text-ellipsis whitespace-nowrap">
          {i.summary}
        </div>
      )}
      <div className="text-muted-foreground flex flex-wrap gap-2.25 font-mono text-[11.5px]">
        <span>
          {r.entries ?? 0} · {entriesWord}
        </span>
        {!!r.attachments && <span>📎 {r.attachments}</span>}
        <span>{fmtBytes(r.bytes)}</span>
        <span>{fmtAge(r.last_entry_at ?? i.updated_at)}</span>
      </div>
    </div>
  )
}
