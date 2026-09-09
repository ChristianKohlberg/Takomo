// Document outline: fold state belongs to the page; transient keyboard focus and
// drag targets stay here. Reordering delegates to the page's validated CRDT history.

import { useEffect, useId, useRef, useState } from 'react'
import { ChevronRight, ChevronsDownUp, ChevronsUpDown } from 'lucide-react'
import { outlineDropPlacement, outlineParent, validOutlineDrop } from '@/lib/outline-interaction'
import type { SectionPlacement, StructureResult } from '@/lib/plan-structure'
import type { PlanSection } from '@/lib/plan-sections'
import { flattenSections, sectionCount, visibleSections } from '@/lib/plan-sections'
import { pendingInSubtree } from '@/lib/plan-proposals'
import type { Standing } from '@/lib/plan-trace'
import { cn } from '@/lib/utils'

export interface OutlineRailLabels {
  /** The outline's own name, on the button that folds it away. */
  outline: string
  expand: string
  collapse: string
  /** What a folded section is holding. `{n}` is the count, at any depth. */
  folded: string
  /** A section nobody has given a title yet. */
  untitled: string
  /** Read out beside the mark. */
  standingConfirmed: string
  standingChanged: string
  standingUnseen: string
  /** What the ◆ beside a row means. `{n}` is how many are waiting. */
  pending: string
  move?: string
}

export interface OutlineRailProps {
  sections: PlanSection[]
  /** The section the column is scrolled to, if any. */
  selected: string | null
  onSelect: (key: string) => void
  /** Absent for read-only viewers. Pointer, keyboard and touch share one action. */
  onMove?: (key: string) => void
  onReorder?: (source: string, target: string, placement: SectionPlacement) => StructureResult
  locale?: 'en' | 'de'
  numbering?: { h1: boolean; h2: boolean }
  /** Section keys this viewer has folded. Never shared. */
  collapsed: ReadonlySet<string>
  onToggle: (key: string) => void
  /** Where each section stands, by key. Absent means the plan has no history yet. */
  standing?: Readonly<Record<string, Standing>>
  /** Proposals waiting on a person, by section key. A FOLDED row reports what
   *  is waiting beneath it too — folding a branch is not a decision to stop
   *  caring what an agent offered inside it. */
  pending?: Readonly<Record<string, number>>
  labels: OutlineRailLabels
  className?: string
}

/** Dot diameter in px by depth. Inline because it is data, not a style — a
 *  class per level would be five classes saying one thing. */
const DOT = [7, 5.5, 4.5, 4]

const dotSize = (depth: number): number => DOT[Math.min(depth, DOT.length - 1)] as number

/** Type quiets with depth, and stops quieting: past level three the indent and
 *  the dot carry it, and shrinking further would just be unreadable. */
const titleClass = (depth: number): string =>
  depth === 0
    ? 'text-[13.5px] font-[620] text-foreground'
    : depth === 1
      ? 'text-[13px] font-medium text-foreground'
      : 'text-[12.5px] text-muted-foreground'

const dotClass = (depth: number, active: boolean): string =>
  active
    ? 'bg-primary'
    : depth === 0
      ? 'bg-foreground/70'
      : depth === 1
        ? 'bg-muted-foreground/70'
        : 'bg-muted-foreground/40'

export function OutlineRail({
  sections,
  selected,
  onSelect,
  onMove,
  onReorder,
  locale = 'en',
  numbering = { h1: true, h2: true },
  collapsed,
  onToggle,
  pending,
  labels,
  className,
}: OutlineRailProps) {
  const rows = visibleSections(sections, collapsed)
  const [focused, setFocused] = useState<string | null>(null)
  const rowElements = useRef(new Map<string, HTMLLIElement>())
  const dragging = useRef<string | null>(null)
  const [source, setSource] = useState<string | null>(null)
  const [drop, setDrop] = useState<{ target: string; placement: SectionPlacement } | null>(null)
  const [announcement, setAnnouncement] = useState('')
  const hold = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number } | null>(null)
  const suppressClick = useRef(false)
  const cancelHold = () => { if (hold.current) clearTimeout(hold.current.timer); hold.current = null }
  useEffect(() => () => { if (hold.current) clearTimeout(hold.current.timer) }, [onMove])
  const branches = flattenSections(sections).filter(section => section.children.length)
  const instructions = useId()
  const de = locale === 'de'
  const focusKey = rows.some(row => row.key === focused) ? focused : rows.some(row => row.key === selected) ? selected : rows[0]?.key
  const clearDrag = () => { dragging.current = null; setSource(null); setDrop(null) }
  function focus(key: string | undefined) {
    if (!key) return
    setFocused(key); rowElements.current.get(key)?.focus()
  }
  const numbered = (section: PlanSection) => section.depth === 0 ? numbering.h1 : section.depth === 1 ? numbering.h2 : true
  const rowLabel = (section: PlanSection) => `${numbered(section) ? section.number + ' ' : ''}${section.title || labels.untitled}`
  function destination(target: PlanSection, placement: SectionPlacement) {
    const relation = placement === 'before' ? (de ? 'Vor' : 'Before') : placement === 'after' ? (de ? 'Nach' : 'After') : (de ? 'Als Unterabschnitt von' : 'As a child of')
    return `${relation} ${rowLabel(target)}`
  }
  function reorder(target: PlanSection, placement: SectionPlacement) {
    const id = dragging.current
    clearDrag()
    if (!id || !onReorder || !validOutlineDrop(sections, id, target.key)) return
    const result = onReorder(id, target.key, placement)
    setAnnouncement(result.ok ? `${de ? 'Verschoben' : 'Moved'}: ${destination(target, placement)}` : de ? 'Das Ziel hat sich geändert. Der Abschnitt wurde nicht verschoben.' : 'The destination changed. The section was not moved.')
    if (result.ok) requestAnimationFrame(() => rowElements.current.get(id)?.focus())
  }


  return (
    // Anchor screen-reader labels here so they cannot extend the page beyond
    // the outline's scroll container and leave blank space below the app.
    <><p id={instructions} className="sr-only">{de ? 'Pfeiltasten navigieren und öffnen oder schließen Abschnitte.' : 'Arrow keys navigate, expand and collapse sections.'} {onReorder && (de ? 'Zeilen zum Verschieben ziehen. Die Zeilenmitte macht den Abschnitt zum Unterabschnitt; die Ränder fügen ihn davor oder danach ein.' : 'Drag rows to move them. The middle makes a child section; the edges insert before or after.')} {onMove && (de ? 'Umschalt+F10 oder langes Drücken öffnet die Zielauswahl.' : 'Shift+F10 or a long press opens the destination picker.')}</p>
    <div className="flex justify-end gap-1">
      <button type="button" className="rounded p-1.5 text-muted-foreground hover:bg-accent disabled:opacity-40" aria-label={de ? 'Alle Abschnitte aufklappen' : 'Expand all sections'} title={de ? 'Alle Abschnitte aufklappen' : 'Expand all sections'} disabled={!branches.some(section => collapsed.has(section.key))} onClick={() => branches.filter(section => collapsed.has(section.key)).forEach(section => onToggle(section.key))}><ChevronsUpDown aria-hidden="true" className="size-4" /></button>
      <button type="button" className="rounded p-1.5 text-muted-foreground hover:bg-accent disabled:opacity-40" aria-label={de ? 'Alle Abschnitte zuklappen' : 'Collapse all sections'} title={de ? 'Alle Abschnitte zuklappen' : 'Collapse all sections'} disabled={!branches.some(section => !collapsed.has(section.key))} onClick={() => branches.filter(section => !collapsed.has(section.key)).forEach(section => onToggle(section.key))}><ChevronsDownUp aria-hidden="true" className="size-4" /></button>
    </div>
    <ul role="tree" aria-label={labels.outline} aria-describedby={instructions} className={cn('relative flex flex-col', className)}>

      {rows.map((section) => {
        const parent = outlineParent(sections, section.key)
        const siblings = parent ? flattenSections(sections).find(row => row.key === parent)!.children : sections
        const active = section.key === selected
        const hasChildren = section.children.length > 0
        const folded = hasChildren && collapsed.has(section.key)
        const hidden = folded ? sectionCount(section) : 0
        const waiting = pending
          ? folded
            ? pendingInSubtree(section, pending)
            : (pending[section.key] ?? 0)
          : 0
        return (
          <li
            key={section.key}
            ref={element => { if (element) rowElements.current.set(section.key, element); else rowElements.current.delete(section.key) }}
            role="treeitem"
            aria-label={rowLabel(section)}
            aria-level={section.depth + 1}
            aria-posinset={siblings.findIndex(row => row.key === section.key) + 1}
            aria-setsize={siblings.length}
            aria-expanded={hasChildren ? !folded : undefined}
            aria-selected={active}
            tabIndex={section.key === focusKey ? 0 : -1}
            onFocus={() => setFocused(section.key)}
            draggable={!!onReorder}
            onPointerDown={event => {
              if (event.pointerType !== 'touch' || !onMove) return
              cancelHold(); suppressClick.current = false
              const key = section.key
              hold.current = { x: event.clientX, y: event.clientY, timer: setTimeout(() => { hold.current = null; suppressClick.current = true; onMove(key) }, 500) }
            }}
            onPointerMove={event => { if (hold.current && Math.hypot(event.clientX - hold.current.x, event.clientY - hold.current.y) > 8) cancelHold() }}
            onPointerUp={cancelHold}
            onPointerCancel={cancelHold}
            onClickCapture={event => { if (suppressClick.current) { event.preventDefault(); event.stopPropagation(); suppressClick.current = false } }}
            onContextMenu={event => { if (onMove) { event.preventDefault(); cancelHold(); clearDrag(); if (!suppressClick.current) onMove(section.key) } }}
            onKeyDown={event => {
              if (event.altKey || event.metaKey || event.ctrlKey) return
              const index = rows.findIndex(row => row.key === section.key)
              if (event.key === 'F10' && event.shiftKey && onMove) { event.preventDefault(); onMove(section.key); return }
              if (event.shiftKey) return
              if (['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End', 'Enter', ' '].includes(event.key)) event.stopPropagation()
              if (event.key === 'ArrowDown') { event.preventDefault(); focus(rows[index + 1]?.key) }
              else if (event.key === 'ArrowUp') { event.preventDefault(); focus(rows[index - 1]?.key) }
              else if (event.key === 'Home') { event.preventDefault(); focus(rows[0]?.key) }
              else if (event.key === 'End') { event.preventDefault(); focus(rows.at(-1)?.key) }
              else if (event.key === 'ArrowRight') { event.preventDefault(); if (folded) onToggle(section.key); else if (hasChildren) focus(section.children[0]?.key) }
              else if (event.key === 'ArrowLeft') { event.preventDefault(); if (hasChildren && !folded) onToggle(section.key); else focus(outlineParent(sections, section.key) ?? undefined) }
              else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onSelect(section.key) }
              else if (event.key === 'Escape') { clearDrag() }
            }}
            onDragStart={event => {
              if (!onReorder) { event.preventDefault(); return }
              dragging.current = section.key; setSource(section.key); setAnnouncement('')
              event.dataTransfer.effectAllowed = 'move'; event.dataTransfer.setData('text/plain', section.key)
            }}
            onDragOver={event => {
              const id = dragging.current
              if (!onReorder || !id || !validOutlineDrop(sections, id, section.key)) { setDrop(null); return }
              event.preventDefault(); event.dataTransfer.dropEffect = 'move'
              const rect = event.currentTarget.getBoundingClientRect()
              const placement = outlineDropPlacement(event.clientY, rect.top, rect.height)
              setDrop({ target: section.key, placement }); setAnnouncement(destination(section, placement))
            }}
            onDragLeave={event => { if (!(event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget))) setDrop(null) }}
            onDrop={event => {
              if (!dragging.current) return
              event.preventDefault()
              const rect = event.currentTarget.getBoundingClientRect()
              reorder(section, outlineDropPlacement(event.clientY, rect.top, rect.height))
            }}
            onDragEnd={clearDrag}
            className={cn(
              'group relative flex items-center gap-1 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-primary',
              onReorder && 'cursor-grab active:cursor-grabbing',
              source === section.key && 'opacity-50',
              drop?.target === section.key && (drop.placement === 'child' ? 'ring-2 ring-primary bg-accent' : drop.placement === 'before' ? 'before:absolute before:inset-x-0 before:top-0 before:border-t-2 before:border-primary' : 'after:absolute after:inset-x-0 after:bottom-0 after:border-b-2 after:border-primary'),
              active ? 'bg-accent' : 'hover:bg-accent/50',
            )}
            // Indentation is data: a depth-4 row needs a depth-4 inset, and
            // Tailwind cannot spell an arbitrary one without a class per level.
            style={{ paddingLeft: `${2 + Math.min(section.depth, 4) * 12}px` }}
          >
            {hasChildren ? (
              <button
                type="button"
                tabIndex={-1}
                aria-label={folded ? labels.expand : labels.collapse}
                aria-expanded={!folded}
                onClick={() => onToggle(section.key)}
                className="text-muted-foreground hover:text-foreground w-4 flex-none text-[10px]"
              >
<ChevronRight aria-hidden="true" className={cn("size-3.5 transition-transform", !folded && "rotate-90")} />
              </button>
            ) : (
              <span className="w-4 flex-none" aria-hidden="true" />
            )}

            <span className="flex w-2.5 flex-none justify-center" aria-hidden="true">
              <span
                className={cn('block rounded-full', dotClass(section.depth, active))}
                style={{
                  width: `${dotSize(section.depth)}px`,
                  height: `${dotSize(section.depth)}px`,
                }}
              />
            </span>

            <button
              type="button"
              tabIndex={-1}
              onClick={() => { focus(section.key); onSelect(section.key) }}
              title={rowLabel(section)}
              aria-label={rowLabel(section)}
              aria-current={active ? 'location' : undefined}
              className={cn("flex min-w-0 grow items-baseline gap-1.5 py-1.5 pr-1 text-left", titleClass(section.depth))}
            >
              {numbered(section) && <span className="text-muted-foreground flex-none font-mono">{section.number}</span>}
              <span
                className={cn(
                  'min-w-0 line-clamp-2 break-words',
                  titleClass(section.depth),
                  section.title ? '' : 'italic opacity-70',
                )}
              >
                {section.title || labels.untitled}
              </span>
            </button>

            {folded && (
              <span
                className="text-muted-foreground flex-none font-mono text-[10px]"
                title={labels.folded.replace('{n}', String(hidden))}
              >
                ⊞ {hidden}
              </span>
            )}
            {waiting > 0 && (
              <span
                className="flex-none px-1 font-mono text-[11px] text-amber-600 dark:text-amber-400"
                title={labels.pending.replace('{n}', String(waiting))}
              >
                <span aria-hidden="true">◆{waiting}</span>
                <span className="sr-only">{labels.pending.replace('{n}', String(waiting))}</span>
              </span>
            )}

          </li>
        )
      })}
    </ul><p role="status" className="sr-only">{announcement}</p></>
  )
}
