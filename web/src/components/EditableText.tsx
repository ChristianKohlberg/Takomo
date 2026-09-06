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
import { cn } from '@/lib/utils'

export interface EditableTextProps {
  value: string
  editable: boolean
  /** Rejecting reverts the visible text to `value`. */
  onCommit: (next: string) => Promise<unknown>
  /** Blank is refused and reverts (a title cannot be empty). */
  required?: boolean
  /**
   * Shown while the field is empty. Rendered through CSS rather than as text, so
   * it can never be committed as a value — a placeholder that saves itself the
   * first time somebody clicks past it is worse than no placeholder.
   */
  placeholder?: string
  className?: string
  as?: 'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6' | 'p'
  onEnter?: () => void
  onArrowUp?: () => boolean
  onArrowDown?: () => boolean
  'aria-label'?: string
}

export function EditableText({
  value,
  editable,
  onCommit,
  required = false,
  placeholder,
  className,
  as = 'p',
  onEnter,
  onArrowUp,
  onArrowDown,
  'aria-label': ariaLabel,
}: EditableTextProps) {
  const ref = useRef<HTMLElement>(null)

  useEffect(() => {
    const node = ref.current
    if (node && node.textContent !== value) node.textContent = value
    // Intentionally only on mount / when the record changes: see the note above
    // about staying uncontrolled while the user types. `as` is in the list
    // because changing the tag (a section moved to another depth renders its
    // heading as h1 -> h2) mounts a fresh, empty element that must be filled.
  }, [value, as])

  const Tag = as

  return (
    <Tag
      ref={ref as never}
      aria-label={ariaLabel}
      data-placeholder={placeholder}
      className={cn(
        placeholder &&
          'empty:before:text-muted-foreground empty:before:content-[attr(data-placeholder)]',
        className,
      )}
      contentEditable={editable}
      suppressContentEditableWarning
      onKeyDown={(event) => {
        if (!editable || event.nativeEvent.isComposing || event.shiftKey || event.altKey || event.ctrlKey || event.metaKey) return
        const selection = window.getSelection()
        if (selection && !selection.isCollapsed) return
        if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
          if (!selection?.rangeCount || !event.currentTarget.contains(selection.anchorNode)) return
          const caret = selection.getRangeAt(0)
          const before = caret.cloneRange()
          before.selectNodeContents(event.currentTarget)
          before.setEnd(caret.startContainer, caret.startOffset)
          const after = caret.cloneRange()
          after.selectNodeContents(event.currentTarget)
          after.setStart(caret.endContainer, caret.endOffset)
          const handled = event.key === 'ArrowUp'
            ? before.toString().length === 0 && onArrowUp?.()
            : after.toString().length === 0 && onArrowDown?.()
          if (handled) event.preventDefault()
        }
        if (event.key === 'Enter' && onEnter) {
          event.preventDefault()
          event.currentTarget.blur()
          onEnter()
        }
      }}
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
