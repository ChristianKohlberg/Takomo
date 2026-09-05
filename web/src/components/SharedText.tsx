import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import * as Y from 'yjs'
import { LOCAL_EDIT, spliceText } from '@/lib/collaboration'
import { cn } from '@/lib/utils'

export interface SharedTextProps {
  text: Y.Text
  label: string
  readOnly?: boolean
  maxLength?: number
  autoFocus?: boolean
  onDone?: () => void
  className?: string
}

/** Bind the field directly to Y.Text; blur never writes an old whole-value draft. */
export function SharedText({ text, label, readOnly, maxLength, autoFocus, onDone, className }: SharedTextProps) {
  const [value, setValue] = useState(() => text.toString())
  const ref = useRef<HTMLTextAreaElement>(null)
  const selection = useRef<{ start: Y.RelativePosition; end: Y.RelativePosition } | null>(null)
  useEffect(() => {
    const before = (transaction: Y.Transaction) => {
      if (transaction.origin === LOCAL_EDIT) return
      const input = ref.current
      if (document.activeElement === input && input) {
        selection.current = {
          start: Y.createRelativePositionFromTypeIndex(text, input.selectionStart),
          end: Y.createRelativePositionFromTypeIndex(text, input.selectionEnd),
        }
      }
    }
    const read = () => setValue(text.toString())
    read()
    text.doc!.on('beforeTransaction', before)
    text.observe(read)
    return () => { text.doc?.off('beforeTransaction', before); text.unobserve(read) }
  }, [text])
  useLayoutEffect(() => {
    const input = ref.current
    const range = selection.current
    if (!range || !input || !text.doc || document.activeElement !== input) return
    const start = Y.createAbsolutePositionFromRelativePosition(range.start, text.doc)
    const end = Y.createAbsolutePositionFromRelativePosition(range.end, text.doc)
    if (start && end) input.setSelectionRange(start.index, end.index)
    selection.current = null
  }, [text, value])
  return <textarea ref={ref} aria-label={label} value={value}
    readOnly={readOnly} maxLength={maxLength} autoFocus={autoFocus}
    onChange={e => { if (!readOnly) spliceText(text, e.target.value) }}
    onBlur={onDone}
    onKeyDown={e => {
      e.stopPropagation()
      if (onDone && (e.key === 'Escape' || (e.key === 'Enter' && !e.shiftKey))) {
        e.preventDefault()
        onDone()
      }
    }}
    className={cn('text-foreground bg-transparent min-w-0 resize-none rounded-md p-2 text-sm leading-relaxed outline-none focus-visible:ring-2 focus-visible:ring-ring', className)} />
}
