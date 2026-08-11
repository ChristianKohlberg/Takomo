import { cn } from '@/lib/utils'

export interface CitationMarkProps {
  /** The reader-facing source number — global across panes, not the author's local index. */
  n: number
  selected?: boolean
  label: string
  onSelect: () => void
}

/**
 * The numbered mark that makes a sentence traceable. Clicking it opens the
 * source's lineage rather than navigating: the point is to check a claim
 * without losing your place in the paragraph.
 */
export function CitationMark({ n, selected, label, onSelect }: CitationMarkProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-label={`${label} ${n}`}
      className={cn(
        'bg-secondary text-secondary-foreground hover:bg-primary hover:text-primary-foreground mx-px cursor-pointer rounded-[3px] px-1 align-super font-mono text-[10px] font-bold',
        selected && 'bg-primary text-primary-foreground',
      )}
    >
      {n}
    </button>
  )
}
