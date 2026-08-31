// The one text caret on the canvas: a thought's TITLE, typed on the thought.
//
// Everything else about a node is a dialog, and that is the rule this component
// exists to keep narrow. A form on the canvas has to fight the canvas for the
// keyboard — Space folds a branch, Enter grows one, Backspace prunes — and the
// old inline editor lost that fight in both directions. One single-line input
// does not: it swallows exactly the keystrokes that are being typed INTO it and
// hands the canvas back the moment it closes.
//
// It earns that place because a modal per new thought is too heavy for the ten
// minutes a brainstorm is for. A node appears where it will live, showing its
// title and a caret, and you type the name straight onto the map.
//
// Enter commits and KEEPS the node selected, so the next Enter on the canvas
// makes the next sibling and the fast loop is genuinely a loop. Tab commits and
// goes a level deeper. Escape abandons — what that means for the node is
// `lib/mindmap-naming.ts`'s decision, not this component's.
import { useEffect, useRef, useState } from 'react'

import { MAX_TITLE } from '@/lib/mindmap-doc'
import { cn } from '@/lib/utils'

/** What happens after a commit: stay on this thought, or open the next one. */
export type NameThen = 'stay' | 'child'

export interface NodeNameInputLabels {
  /** The caret's accessible name — it has no visible label on a canvas. */
  field: string
  /** Shown while it is empty. */
  hint: string
}

export interface NodeNameInputProps {
  /** The title to start from, selected so a placeholder is typed over. */
  value: string
  onCommit: (title: string, then: NameThen) => void
  onCancel: () => void
  labels: NodeNameInputLabels
  className?: string
}

export function NodeNameInput({ value, onCommit, onCancel, labels, className }: NodeNameInputProps) {
  // A draft, never a per-keystroke write: this is one shared history, and a name
  // typed slowly would arrive at a collaborator letter by letter.
  const [draft, setDraft] = useState(value)
  const ref = useRef<HTMLInputElement | null>(null)
  // The caret has exactly one ending. Blur fires on the way out of a commit as
  // well, and a second commit would write a title the person never confirmed.
  const settled = useRef(false)

  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])

  const commit = (then: NameThen) => {
    if (settled.current) return
    settled.current = true
    onCommit(draft, then)
  }
  const cancel = () => {
    if (settled.current) return
    settled.current = true
    onCancel()
  }

  return (
    <input
      ref={ref}
      type="text"
      aria-label={labels.field}
      placeholder={labels.hint}
      value={draft}
      maxLength={MAX_TITLE}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => commit('stay')}
      // Nothing that happens in here belongs to the canvas. Without this, typing
      // `Delete` prunes the branch being named, a press pans the map, and the
      // wheel zooms it. ⌘K is unaffected: it listens in the capture phase and
      // refuses to OPEN over a text field of its own accord.
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') {
          e.preventDefault()
          commit('stay')
        } else if (e.key === 'Tab') {
          e.preventDefault()
          commit('child')
        } else if (e.key === 'Escape') {
          e.preventDefault()
          cancel()
        }
      }}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
      className={cn(
        'border-ring bg-card text-foreground w-full rounded-sm border px-1 py-0.5 text-[12.5px] leading-snug outline-none',
        className,
      )}
    />
  )
}
