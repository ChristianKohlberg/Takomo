// Right-click a thought.
//
// The pill above the selected node carries the three or four verbs somebody
// reaches for constantly; this carries the occasional ones, and it is the place
// REMOVING a node lives. That split is the whole point: a delete on the pill is
// a delete one pixel from the verbs used most, and this map is shared, so the
// branch that vanishes was very likely written by somebody else.
//
// It is not right-click only. A context menu reachable by exactly one gesture no
// keyboard has is a set of commands a keyboard user does not have — so the
// canvas opens the same menu on Shift+F10 and on the ContextMenu key, anchored
// on the selected node, and the menu takes focus and answers to arrows and
// Escape either way.
import { useEffect, useRef } from 'react'

import { cn } from '@/lib/utils'

export interface MenuItem {
  id: string
  label: string
  /** Drawn apart and in the destructive colour. At most one, and it is last. */
  danger?: boolean
}

export interface NodeMenuProps {
  items: readonly MenuItem[]
  /** Where to draw it, in pixels within the canvas container. Already clamped by
   *  the caller, which is the only thing that knows the container's size. */
  at: { x: number; y: number }
  ariaLabel: string
  onRun: (id: string) => void
  onClose: () => void
  className?: string
}

export function NodeMenu({ items, at, ariaLabel, onRun, onClose, className }: NodeMenuProps) {
  const ref = useRef<HTMLDivElement | null>(null)

  // Take the focus on open. Without this the menu is drawn beside a canvas that
  // still owns the keyboard, so Escape would close nothing and an arrow key
  // would pan.
  useEffect(() => {
    const first = ref.current?.querySelector('button')
    first?.focus()
  }, [])

  // A press anywhere else dismisses it — including on the canvas, whose own
  // pointer handler would otherwise select a node behind the open menu.
  useEffect(() => {
    const onDown = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) onClose()
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [onClose])

  const move = (from: HTMLElement, delta: number) => {
    const buttons = [...(ref.current?.querySelectorAll('button') ?? [])]
    const index = buttons.indexOf(from as HTMLButtonElement)
    const next = buttons[(index + delta + buttons.length) % buttons.length]
    next?.focus()
  }

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={ariaLabel}
      style={{ left: `${at.x}px`, top: `${at.y}px` }}
      onKeyDown={(e) => {
        // Every key stops here: the canvas keyboard grows and folds the map, and
        // none of that may happen while a menu is open over it.
        e.stopPropagation()
        if (e.key === 'Escape') {
          e.preventDefault()
          onClose()
        } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault()
          move(e.target as HTMLElement, e.key === 'ArrowDown' ? 1 : -1)
        } else if (e.key === 'Tab') {
          onClose()
        }
      }}
      className={cn(
        'bg-card border-border absolute z-20 flex w-56 max-w-[calc(100vw-2rem)] flex-col rounded-lg border py-1 shadow-lg',
        className,
      )}
    >
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          onClick={() => onRun(item.id)}
          className={cn(
            'hover:bg-accent focus:bg-accent cursor-pointer px-3 py-1.5 text-left text-[12.5px] outline-none',
            item.danger && 'text-destructive border-t-border-soft mt-1 border-t pt-2',
          )}
        >
          {item.label}
        </button>
      ))}
    </div>
  )
}
