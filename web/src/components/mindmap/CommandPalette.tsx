// ⌘K: everything the mindmap can do, in one list, scoped to what is selected.
//
// The map used to carry its affordances as permanent chrome — a rail, a header
// of buttons, a side panel. On a canvas that is backwards: the chrome is always
// there and the thing you came for is never where your hands are. So the surface
// is the map, and this is how you reach the rest.
//
// It is deliberately a dumb list. The caller decides WHICH commands exist (see
// `lib/mindmap-commands.ts`, where the rule is testable) and what a row is
// called; this draws them, moves a selection with the keyboard, and reports the
// one that was run. That split is the reason the command set can be tested at
// all — jsdom has no layout engine and could prove nothing about the overlay.
//
// Two rules it does hold itself to:
//
//   * The row a keystroke would run is the row highlighted. `active` is an ID
//     rather than an index, so filtering as you type cannot silently move the
//     highlight onto a different command.
//   * Escape and ⌘K both close from INSIDE the input. The page-level shortcut
//     refuses to fire while a text field has focus — which is right for opening
//     and wrong for closing, so closing is handled here.
import { useEffect, useRef, type ReactNode } from 'react'

import { cn } from '@/lib/utils'

export interface PaletteItem {
  id: string
  label: string
  /** One line of what it does, or where it goes. */
  hint?: string
}

export interface CommandPaletteLabels {
  /** Above the input: what the list is scoped to. */
  scopeNode: string
  scopeMap: string
  placeholder: string
  noMatch: string
  /** The keyboard legend along the bottom. */
  keys: string
}

export interface CommandPaletteProps {
  /** What the palette is scoped to — the selected node's title, or the map's. */
  scope: string
  scopeKind: 'node' | 'map'
  items: readonly PaletteItem[]
  query: string
  onQuery: (query: string) => void
  /** The id of the row that would run — the caller owns it so a second stage
   *  (go to node…, switch project…) can reset it. */
  active: string | null
  onActive: (id: string) => void
  onRun: (id: string) => void
  onClose: () => void
  labels: CommandPaletteLabels
  /** An optional line under the input — a second stage says what it is asking. */
  children?: ReactNode
  className?: string
}

export function CommandPalette({
  scope,
  scopeKind,
  items,
  query,
  onQuery,
  active,
  onActive,
  onRun,
  onClose,
  labels,
  children,
  className,
}: CommandPaletteProps) {
  const activeId = items.some((i) => i.id === active) ? active : (items[0]?.id ?? null)
  const activeRef = useRef<HTMLLIElement | null>(null)

  // Keep the highlighted row visible while arrowing through a long list. A
  // palette that scrolls its own selection off screen is one you have to look
  // away from to use.
  useEffect(() => {
    // Optional twice over: jsdom does not implement scrollIntoView, and a test
    // that renders this must not die on a convenience.
    activeRef.current?.scrollIntoView?.({ block: 'nearest' })
  }, [activeId])

  const step = (delta: number) => {
    if (items.length === 0) return
    const at = items.findIndex((i) => i.id === activeId)
    const next = items[(at + delta + items.length) % items.length]
    if (next) onActive(next.id)
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/25 px-4 pt-[12vh]"
      onPointerDown={(e) => {
        // A click on the backdrop closes; a click inside the panel must not.
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={scope}
        className={cn(
          'bg-card border-border flex max-h-[70vh] w-full max-w-[calc(100vw-2rem)] flex-col overflow-hidden rounded-xl border shadow-lg md:max-w-xl',
          className,
        )}
      >
        <div className="border-b-border-soft flex flex-col gap-1 border-b px-3 pt-2.5 pb-2">
          <div className="text-muted-foreground flex items-center gap-1.5 text-[11px] font-[700] tracking-wide uppercase">
            <span>{scopeKind === 'node' ? labels.scopeNode : labels.scopeMap}</span>
            <span className="text-foreground min-w-0 truncate normal-case">{scope}</span>
          </div>
          <input
            autoFocus
            value={query}
            placeholder={labels.placeholder}
            aria-label={labels.placeholder}
            onChange={(e) => onQuery(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation()
              if (e.key === 'Escape' || (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey))) {
                e.preventDefault()
                onClose()
              } else if (e.key === 'ArrowDown') {
                e.preventDefault()
                step(1)
              } else if (e.key === 'ArrowUp') {
                e.preventDefault()
                step(-1)
              } else if (e.key === 'Enter' && activeId) {
                e.preventDefault()
                onRun(activeId)
              }
            }}
            className="text-foreground w-full bg-transparent text-[14px] outline-none"
          />
          {children}
        </div>

        {items.length === 0 ? (
          <div className="text-muted-foreground px-3 py-6 text-center text-[12.5px]">
            {labels.noMatch}
          </div>
        ) : (
          <ul className="m-0 min-h-0 list-none overflow-y-auto p-1">
            {items.map((item) => (
              <li
                key={item.id}
                ref={item.id === activeId ? activeRef : null}
                aria-selected={item.id === activeId}
              >
                <button
                  type="button"
                  onPointerEnter={() => onActive(item.id)}
                  onClick={() => onRun(item.id)}
                  className={cn(
                    'flex w-full cursor-pointer items-baseline gap-2 rounded-lg px-2.5 py-1.5 text-left',
                    item.id === activeId && 'bg-accent',
                  )}
                >
                  <span className="text-foreground shrink-0 text-[13px] font-[650]">
                    {item.label}
                  </span>
                  {item.hint && (
                    <span className="text-muted-foreground min-w-0 truncate text-[11.5px]">
                      {item.hint}
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="border-t-border-soft text-muted-foreground border-t px-3 py-1.5 text-[11px]">
          {labels.keys}
        </div>
      </div>
    </div>
  )
}
