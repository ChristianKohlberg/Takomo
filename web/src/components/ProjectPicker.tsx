// Searchable project scope selector for the header and standalone navigation rail.
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { ChevronsUpDownIcon, LayersIcon, SearchIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import { rankOptions } from '@/lib/typeahead'
import { Hint } from '@/components/Hint'

export interface ProjectOption {
  id: string
  /** The project's display name, when it has one. Searched alongside the id. */
  name?: string | null
  /**
   * Archived projects stay in `projects` so the trigger can label an existing
   * scope, but they are dropped from the pick list — see `selectableProjects`.
   */
  archived?: boolean
  archived_at?: string | null
}

/** Matches the API's `archived` boolean; `archived_at` is the timestamp twin. */
function isArchived(p: ProjectOption): boolean {
  return p.archived === true || p.archived_at != null
}

export interface ProjectPickerLabels {
  /** Accessible name for the trigger, e.g. "project". */
  project: string
  /** Placeholder in the search field. */
  search: string
  /** Shown when nothing matches; `{q}` is the query. */
  noMatch: string
  /**
   * The "no project chosen" entry, meaning ALL projects. Omit it and the picker
   * offers no such entry — which is what /board needs, because a kanban's
   * columns come from one project's workflow and two projects need not agree.
   */
  all?: string
}

export interface ProjectPickerProps {
  projects: ProjectOption[]
  /** The chosen project id; `''` is "all projects" where that is offered. */
  value: string
  onChange: (id: string) => void
  labels: ProjectPickerLabels
  /** Icon-only, for a collapsed rail. */
  collapsed?: boolean
  /** Responsive header trigger with its popover below. */
  header?: boolean
}

/** Enough to fill the popover; the search field is how you reach the rest. */
const MAX_SHOWN = 12

export function ProjectPicker({
  projects,
  value,
  onChange,
  labels,
  collapsed = false,
  header = false,
}: ProjectPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const boxRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const listId = useId()
  const optId = (i: number) => `${listId}-opt-${i}`

  // Filter here, not at each NavRail mount: every surface passes the full
  // `listProjects` payload (including archived rows) so a scope that was frozen
  // after you picked it still labels the trigger — criteria 2 — while the list
  // and search only offer projects you can still work in.
  const selectableProjects = useMemo(() => projects.filter((p) => !isArchived(p)), [projects])
  const options = useMemo(
    () => selectableProjects.map((p) => ({ id: p.id, title: p.name ?? null })),
    [selectableProjects],
  )
  const { shown, total } = useMemo(() => rankOptions(options, query, MAX_SHOWN), [options, query])

  // The "all projects" row is an option too — it takes index 0 so the arrow keys
  // reach it. Without that it would be clickable but not keyboard-reachable,
  // which is the classic half-accessible combobox.
  const rows = useMemo(
    () => (labels.all != null ? [{ id: '', title: labels.all }, ...shown] : shown),
    [labels.all, shown],
  )

  useEffect(() => setActive(0), [query])

  useEffect(() => {
    if (!open) return
    // Opening puts the caret in the search field: the whole point of the
    // control is that you can type immediately.
    inputRef.current?.focus()
  }, [open])

  useEffect(() => {
    function onDoc(e: MouseEvent) {
      if (boxRef.current && !boxRef.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDoc)
    return () => document.removeEventListener('mousedown', onDoc)
  }, [])

  const current = projects.find((p) => p.id === value)
  const currentLabel = value ? (current?.name ?? value) : (labels.all ?? labels.project)
  const initial = (value ? currentLabel : '').trim().charAt(0).toUpperCase()

  function take(id: string) {
    onChange(id)
    setQuery('')
    setOpen(false)
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActive((i) => Math.min(i + 1, rows.length - 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActive((i) => Math.max(i - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const pick = rows[active]
      if (pick) take(pick.id)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      setQuery('')
      setOpen(false)
    }
  }

  return (
    <div ref={boxRef} className="relative">
      <Hint text={(collapsed || header) ? `${labels.project}: ${currentLabel}` : undefined}>
        <button
          type="button"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-label={`${labels.project}: ${currentLabel}`}
          onClick={() => setOpen((o) => !o)}
          className={cn(
            'border-border hover:border-ring hover:text-primary text-foreground flex cursor-pointer items-center rounded-lg border text-[13px] font-[650]',
            header ? 'h-10 max-w-48 gap-2 px-2.5' : collapsed
              ? 'size-10 justify-center'
              : 'bg-muted w-full gap-2 px-2.5 py-1.5 text-left',
          )}
        >
          {header ? (
            <>
              <LayersIcon size={17} className="flex-none" />
              <span className="hidden min-w-0 truncate lg:block">{currentLabel}</span>
              <ChevronsUpDownIcon size={14} className="text-muted-foreground flex-none" />
            </>
          ) : collapsed ? (
            // An initial carries more than a generic icon once you have picked
            // something; the icon is only for the unscoped state.
            initial ? <span>{initial}</span> : <LayersIcon size={17} />
          ) : (
            <>
              <span className="min-w-0 grow truncate">{currentLabel}</span>
              <ChevronsUpDownIcon size={14} className="text-muted-foreground flex-none" />
            </>
          )}
        </button>
      </Hint>

      {open && (
        <div
          className={cn(
            'bg-card border-border absolute z-50 max-h-80 w-64 max-w-[calc(100vw-2rem)] overflow-hidden rounded-lg border shadow-[var(--shadow)]',
            // Collapsed, the rail is 56px wide and a popover under the trigger
            // would hang off it; beside it is the only place it fits.
            collapsed && !header ? 'top-0 left-full ml-1' : 'top-full left-0 mt-1',
            header && 'max-w-[calc(100vw-8rem)]',
          )}
        >
          <div className="border-b-border-soft flex items-center gap-1.5 border-b px-2.5 py-1.5">
            <SearchIcon size={14} className="text-muted-foreground flex-none" />
            <input
              ref={inputRef}
              role="combobox"
              aria-expanded
              aria-controls={listId}
              aria-activedescendant={rows[active] ? optId(active) : undefined}
              aria-autocomplete="list"
              aria-label={labels.search}
              placeholder={labels.search}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onKeyDown}
              className="w-full bg-transparent text-base outline-none md:text-[13px]"
            />
          </div>

          <div id={listId} role="listbox" aria-label={labels.project} className="max-h-64 overflow-y-auto py-1">
            {rows.map((row, i) => (
              <button
                key={row.id || ' all'}
                id={optId(i)}
                type="button"
                role="option"
                aria-selected={row.id === value}
                onMouseEnter={() => setActive(i)}
                onClick={() => take(row.id)}
                className={cn(
                  'flex w-full cursor-pointer flex-col px-3 py-1.5 text-left',
                  i === active && 'bg-accent',
                  row.id === value && 'text-primary font-[680]',
                )}
              >
                <span className="truncate text-[13px]">{row.title ?? row.id}</span>
                {/* The id is what every API call and every ticket prefix
                    uses, so it stays visible even when a name exists. */}
                {row.id !== '' && row.title && (
                  <span className="text-muted-foreground font-mono text-[11px]">{row.id}</span>
                )}
              </button>
            ))}

            {/* Keyed off `shown`, not `rows`: the all-projects entry is always
                present, so a `rows.length === 0` test could never fire and a
                fruitless search would look like a list with one result in it. */}
            {query.trim() !== '' && shown.length === 0 && (
              <div className="text-muted-foreground px-3 py-2 text-[12.5px]">
                {labels.noMatch.replace('{q}', query)}
              </div>
            )}
          </div>

          {total > shown.length && (
            <div className="text-muted-foreground border-t-border-soft border-t px-3 py-1 text-[11.5px]">
              {`${shown.length} / ${total}`}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
