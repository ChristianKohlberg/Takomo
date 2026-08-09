// One typeahead, mounted wherever a filter needs one.
//
// A combobox rather than a <select>: a project can hold hundreds of tickets and
// a dropdown of those is unusable, but the control it replaced was keyboard
// operable and this has to stay so — arrow keys move the active option, Enter
// takes it, Escape clears. A filter you can only reach with a mouse is a filter
// a keyboard reader cannot use at all.
//
// ONE component, several mount points: /board's ticket filter and its tag-value
// filter, and /inbox's ticket filter. That is the decision worth keeping — if
// they fork, the ARIA and keyboard guarantees stop covering whichever copy was
// not maintained, and nothing would say so.
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

export interface TypeaheadOption {
  id: string
  /** Secondary line; the id is always shown in mono above it. */
  title?: string | null
}

export interface TypeaheadProps {
  /** DOM id of the mount — the several filters are told apart by it. */
  id: string
  options: TypeaheadOption[]
  value: string
  onChange: (id: string) => void
  labels: {
    all: string
    placeholder: string
    clear: string
    noMatch: string
    count: string
    count1: string
    /** Shown when the list is truncated; `{shown}` of `{n}`. Falls back to `count`. */
    countTruncated?: string
  }
}

const MAX_SHOWN = 12

export function Typeahead({ id: mountId, options, value, onChange, labels }: TypeaheadProps) {
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const listId = useId()
  const boxRef = useRef<HTMLDivElement>(null)
  // Each option needs a stable id so the input can point at the active one:
  // `aria-activedescendant` is how a screen reader is told which option the
  // arrow keys are on, and without it the popup is navigable only by sight.
  const optId = (i: number) => `${listId}-opt-${i}`

  // Rank, then truncate — and count BEFORE truncating.
  //
  // This used to `.filter().slice(0, 12)` and then report `matches.length`, so
  // the footer said "12 matches" whether there were 12 or 400: the reader was
  // told the list was complete when it was a fraction of it. Worse, with no
  // ranking the survivors were just the first twelve in server order, so on a
  // large project the ticket you wanted could be unreachable no matter what you
  // typed — narrowing further needs text you cannot see.
  //
  // The ranking is deliberately cheap and predictable: an exact id wins, then
  // an id prefix, then a title prefix, then anything else. No fuzzy matching —
  // a filter that reorders unpredictably is worse than one that does not.
  const { all, shown } = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return { all: options, shown: options.slice(0, MAX_SHOWN) }
    const hits = options.filter(
      (t) => t.id.toLowerCase().includes(q) || (t.title ?? '').toLowerCase().includes(q),
    )
    const rank = (t: TypeaheadOption) => {
      const id = t.id.toLowerCase()
      const title = (t.title ?? '').toLowerCase()
      if (id === q) return 0
      if (id.startsWith(q)) return 1
      if (title.startsWith(q)) return 2
      if (id.includes(q)) return 3
      return 4
    }
    // A stable sort keeps server order within a rank, so equal-ranked results
    // do not shuffle as you type.
    const ranked = [...hits].sort((a, b) => rank(a) - rank(b))
    return { all: ranked, shown: ranked.slice(0, MAX_SHOWN) }
  }, [options, query])

  const matches = shown

  useEffect(() => setActive(0), [query])

  // Clicking away closes it; without this the list survives a click on the
  // question it was used to find.
  useEffect(() => {
    function onDoc(e: MouseEvent) {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [])

  const label = value
    ? (options.find((t) => t.id === value)?.title ?? value)
    : labels.all

  function take(id: string) {
    onChange(id)
    setQuery('')
    setOpen(false)
  }

  return (
    <div ref={boxRef} id={mountId} className="relative">
      <input
        role="combobox"
        aria-expanded={open}
        aria-controls={listId}
        aria-activedescendant={open && matches[active] ? optId(active) : undefined}
        aria-autocomplete="list"
        aria-label={labels.placeholder}
        placeholder={value ? label : labels.placeholder}
        value={query}
        onFocus={() => setOpen(true)}
        onChange={(e) => {
          setQuery(e.target.value)
          setOpen(true)
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown') {
            e.preventDefault()
            setOpen(true)
            setActive((i) => Math.min(i + 1, matches.length - 1))
          } else if (e.key === 'ArrowUp') {
            e.preventDefault()
            setActive((i) => Math.max(i - 1, 0))
          } else if (e.key === 'Enter') {
            e.preventDefault()
            const pick = matches[active]
            if (pick) take(pick.id)
          } else if (e.key === 'Escape') {
            e.preventDefault()
            setQuery('')
            setOpen(false)
            if (value) onChange('')
          }
        }}
        className={cn(
          'bg-muted border-border focus:border-ring w-full sm:w-55 rounded-lg border px-2.5 py-1.5 text-base md:text-[13px] outline-none',
          value && 'text-primary font-[650]',
        )}
      />

      {value && (
        <button
          type="button"
          className="ta-clear text-muted-foreground hover:text-primary absolute top-1.5 right-2 cursor-pointer"
          title={labels.clear}
          aria-label={labels.clear}
          onClick={() => take('')}
        >
          ×
        </button>
      )}

      {open && (
        <div
          id={listId}
          role="listbox"
          className="bg-card border-border absolute top-full left-0 max-w-[calc(100vw-2rem)] z-50 mt-1 max-h-72 w-80 overflow-y-auto rounded-lg border shadow-[var(--shadow)]"
        >
          <button
            type="button"
            role="option"
            aria-selected={!value}
            onClick={() => take('')}
            className="hover:bg-muted w-full cursor-pointer px-3 py-1.5 text-left text-[13px]"
          >
            {labels.all}
          </button>
          {matches.length === 0 ? (
            <div className="text-muted-foreground px-3 py-2 text-[12.5px]">
              {labels.noMatch.replace('{q}', query)}
            </div>
          ) : (
            <>
              {matches.map((t, i) => (
                <button
                  key={t.id}
                  id={optId(i)}
                  type="button"
                  role="option"
                  aria-selected={t.id === value}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => take(t.id)}
                  className={cn(
                    'flex w-full cursor-pointer flex-col px-3 py-1.5 text-left',
                    i === active && 'bg-accent',
                  )}
                >
                  <span className="font-mono text-[11.5px]">{t.id}</span>
                  {t.title && <span className="truncate text-[13px]">{t.title}</span>}
                </button>
              ))}
              <div className="text-muted-foreground border-t-border-soft border-t px-3 py-1 text-[11.5px]">
                {(all.length === 1
                  ? labels.count1
                  : all.length > matches.length
                    ? (labels.countTruncated ?? labels.count)
                    : labels.count
                )
                  .replace('{shown}', String(matches.length))
                  .replace('{n}', String(all.length))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}
