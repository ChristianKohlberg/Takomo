import { CITE_ATTR } from '@/lib/initiative-anchor'
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
 *
 * It DISPLAYS a bare number but the prose it came from says `[3]`, so it carries
 * its source form in a data attribute. That is what lets a text selection
 * dragged across a mark be converted into an offset in the same coordinate space
 * the prose, the anchors and the diff all use — see lib/initiative-anchor.ts.
 */
export function CitationMark({ n, selected, label, onSelect }: CitationMarkProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      {...{ [CITE_ATTR]: `[${n}]` }}
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
