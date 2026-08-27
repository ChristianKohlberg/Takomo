// ⌘K — the contextual menu, and the prompt bar it doubles as.
//
// Three decisions, all from the doctest prototype and all worth keeping:
//
// **The filter box is also the instruction.** Type "tighten this" and no entry
// matches; the menu offers to run exactly that instead of showing "no results".
// A command menu that can be a dead end is one people stop opening.
//
// **What is offered depends on what is selected.** A highlighted sentence gets
// different actions from a heading, and a heading gets different ones from an
// empty caret. That is what makes the menu teach the model behind it rather than
// just being a text box with a shortcut.
//
// **Choosing runs it immediately.** There is no confirm step, because the result
// is already a proposal nobody has accepted — a second "are you sure?" for the
// same thing is friction protecting nothing.
import { useEffect, useMemo, useRef, useState } from 'react'

export interface Suggestion {
  /** What the user reads. */
  label: string
  /** What the model is told. */
  instruction: string
  /** One line under the label. */
  hint?: string
}

export interface CommandMenuLabels {
  placeholder: string
  runFreeText: string
  scoped: string
  whole: string
  close: string
  running: string
}

export interface CommandMenuProps {
  open: boolean
  onClose: () => void
  /** The block the caret is in, if any — what a scoped run targets. */
  scopeId: string | null
  /** The selected words, quoted into the instruction when present. */
  quote: string
  suggestions: readonly Suggestion[]
  busy: boolean
  onRun: (instruction: string) => void
  labels: CommandMenuLabels
}

export function CommandMenu({
  open,
  onClose,
  scopeId,
  quote,
  suggestions,
  busy,
  onRun,
  labels,
}: CommandMenuProps) {
  const [filter, setFilter] = useState('')
  const [active, setActive] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (open) {
      setFilter('')
      setActive(0)
      // A frame's delay: the input does not exist until this render commits.
      requestAnimationFrame(() => inputRef.current?.focus())
    }
  }, [open])

  const matches = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    if (!needle) return suggestions
    return suggestions.filter(
      (s) =>
        s.label.toLowerCase().includes(needle) ||
        s.instruction.toLowerCase().includes(needle),
    )
  }, [filter, suggestions])

  // The free-text escape hatch: whatever was typed, offered as its own entry
  // when nothing matches it.
  const freeText = filter.trim()
  const showFree = freeText.length > 0 && matches.length === 0

  if (!open) return null

  const run = (instruction: string) => {
    if (!instruction.trim() || busy) return
    onRun(instruction.trim())
  }

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActive((i) => Math.min(i + 1, Math.max(matches.length - 1, 0)))
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActive((i) => Math.max(i - 1, 0))
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      if (showFree) run(freeText)
      else if (matches[active]) run(matches[active].instruction)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/30 px-4 pt-[12vh]"
      // A click on the backdrop closes. The dialog itself stops propagation.
      onClick={onClose}
      role="presentation"
    >
      <div
        className="bg-card border-border-soft w-full max-w-lg overflow-hidden rounded-lg border shadow-lg"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={labels.placeholder}
      >
        <div className="border-b-border-soft flex items-center gap-2 border-b px-3 py-2.5">
          <input
            ref={inputRef}
            value={filter}
            disabled={busy}
            onChange={(e) => {
              setFilter(e.target.value)
              setActive(0)
            }}
            onKeyDown={onKeyDown}
            placeholder={labels.placeholder}
            className="min-w-0 grow bg-transparent text-[14px] outline-none"
          />
          <span className="text-muted-foreground flex-none text-[11px]">
            {scopeId ? `${labels.scoped} ${scopeId}` : labels.whole}
          </span>
        </div>

        {quote && (
          <p className="text-muted-foreground border-b-border-soft border-b px-3 py-1.5 text-[11.5px] italic">
            “{quote.length > 120 ? quote.slice(0, 120) + '…' : quote}”
          </p>
        )}

        <ul className="max-h-[46vh] overflow-y-auto py-1">
          {showFree ? (
            <li>
              <button
                type="button"
                disabled={busy}
                onClick={() => run(freeText)}
                className="hover:bg-accent/60 bg-accent flex w-full flex-col items-start px-3 py-2 text-left"
              >
                <span className="text-[13.5px]">{labels.runFreeText}</span>
                <span className="text-muted-foreground text-[12px]">“{freeText}”</span>
              </button>
            </li>
          ) : (
            matches.map((s, i) => (
              <li key={s.label}>
                <button
                  type="button"
                  disabled={busy}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => run(s.instruction)}
                  className={
                    'flex w-full flex-col items-start px-3 py-2 text-left ' +
                    (i === active ? 'bg-accent' : 'hover:bg-accent/60')
                  }
                >
                  <span className="text-[13.5px]">{s.label}</span>
                  {s.hint && (
                    <span className="text-muted-foreground text-[12px]">{s.hint}</span>
                  )}
                </button>
              </li>
            ))
          )}
        </ul>

        {busy && (
          <p className="text-muted-foreground border-t-border-soft border-t px-3 py-2 text-[12px]">
            {labels.running}
          </p>
        )}
      </div>
    </div>
  )
}
