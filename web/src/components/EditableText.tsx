// Text that is edited in place and saved on blur.
//
// Retitling an initiative is the smallest possible change, and a modal for it
// would be heavier than the edit — so the original page used `contenteditable`
// and this keeps that. Two rules make it safe with React:
//
//   - It is UNCONTROLLED. React writes the initial text once; after that the DOM
//     owns it, so a re-render mid-typing cannot move the caret.
//   - The caller passes `key={id}` to remount when the underlying record
//     changes, which is what re-syncs the text.
//
// `onCommit` returns a promise; if it rejects, the text is put back — the same
// revert-on-failure the page had.
import { useEffect, useRef } from 'react'

export interface EditableTextProps {
  value: string
  editable: boolean
  /** Rejecting reverts the visible text to `value`. */
  onCommit: (next: string) => Promise<unknown>
  /** Blank is refused and reverts (a title cannot be empty). */
  required?: boolean
  className?: string
  as?: 'h1' | 'p'
  'aria-label'?: string
}

export function EditableText({
  value,
  editable,
  onCommit,
  required = false,
  className,
  as = 'p',
  'aria-label': ariaLabel,
}: EditableTextProps) {
  const ref = useRef<HTMLElement>(null)

  useEffect(() => {
    const node = ref.current
    if (node && node.textContent !== value) node.textContent = value
    // Intentionally only on mount / when the record changes: see the note above
    // about staying uncontrolled while the user types.
  }, [value])

  const Tag = as

  return (
    <Tag
      ref={ref as never}
      aria-label={ariaLabel}
      className={className}
      contentEditable={editable}
      suppressContentEditableWarning
      onBlur={() => {
        const node = ref.current
        if (!node) return
        const next = (node.textContent ?? '').trim()
        if (required && !next) {
          node.textContent = value
          return
        }
        if (next === value) return
        onCommit(next).catch(() => {
          if (ref.current) ref.current.textContent = value
        })
      }}
    />
  )
}
