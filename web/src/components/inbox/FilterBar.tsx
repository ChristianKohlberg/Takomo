// The /inbox filter bar — its own section, directly above the list.
//
// These controls used to live in the header, beside the project picker and the
// sign-out button. Two problems with that. The header is shared chrome, so a
// filter sitting in it reads as global state when it only scopes THIS surface;
// and the header already wraps at 375px, so on a phone the ticket filter landed
// on a second row that pushed the list further down the one screen it has.
//
// Given its own row the bar can also say what it is doing — how many questions
// survive the filters, and a way to drop them — which a control tucked into the
// header had no room to.
import { Typeahead, type TypeaheadOption } from '../Typeahead'
import { cn } from '@/lib/utils'

export interface FilterBarLabels {
  filters: string
  /** Ticket typeahead. */
  allTickets: string
  taTicket: string
  taClear: string
  taNoMatch: string
  taCount: string
  taCount1: string
  taCountMore: string
  /** Free-text search. */
  search: string
  searchPlaceholder: string
  /** Result count: `{n}` questions. */
  count: string
  count1: string
  clearAll: string
}

export interface FilterBarProps {
  tickets: TypeaheadOption[]
  ticket: string
  onTicket: (id: string) => void
  search: string
  onSearch: (text: string) => void
  /** How many questions survive the filters, across every folder. */
  matched: number
  /** Whether the phone's collapsed bank is expanded. */
  open: boolean
  onOpen: (open: boolean) => void
  labels: FilterBarLabels
  className?: string
}

export function FilterBar({
  tickets,
  ticket,
  onTicket,
  search,
  onSearch,
  matched,
  open,
  onOpen,
  labels,
  className,
}: FilterBarProps) {
  const active = (ticket ? 1 : 0) + (search.trim() ? 1 : 0)

  return (
    <section
      aria-label={labels.filters}
      className={cn(
        'bg-card border-b-border-soft flex flex-none flex-wrap items-center gap-2.5 border-b px-5 py-2.5',
        className,
      )}
    >
      {/* The bank collapses on a phone, the way /board's does — the list is what
          a reader came for, and a permanently open filter row costs it two rows
          of a screen that has about seven. */}
      <button
        type="button"
        onClick={() => onOpen(!open)}
        aria-expanded={open}
        className="text-muted-foreground border-border cursor-pointer rounded-lg border px-3 py-2 text-[13px] font-[650] md:hidden"
      >
        {labels.filters}
        {active > 0 && (
          <span className="bg-primary text-primary-foreground ml-1.5 inline-block min-w-[17px] rounded-[9px] px-1.25 text-center text-[11px] font-bold">
            {active}
          </span>
        )}
      </button>

      <div
        className={cn(
          'flex flex-wrap items-center gap-2.5',
          open ? 'flex w-full md:w-auto' : 'hidden md:flex',
        )}
      >
        <Typeahead
          id="tickpick"
          options={tickets}
          value={ticket}
          onChange={onTicket}
          labels={{
            all: labels.allTickets,
            placeholder: labels.taTicket,
            clear: labels.taClear,
            noMatch: labels.taNoMatch,
            count: labels.taCount,
            count1: labels.taCount1,
            countTruncated: labels.taCountMore,
          }}
        />

        {/* `search`, not `text`: it gets the browser's clear affordance and the
            phone keyboard's search key for free. */}
        <input
          type="search"
          aria-label={labels.search}
          placeholder={labels.searchPlaceholder}
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          className="bg-muted text-foreground border-border focus:border-ring w-full min-w-0 rounded-lg border px-3 py-1.5 text-[13px] outline-none md:w-64"
        />

        {active > 0 && (
          <button
            type="button"
            onClick={() => {
              onTicket('')
              onSearch('')
            }}
            className="text-primary cursor-pointer px-2 py-1.5 text-[13px] font-[650] underline"
          >
            {labels.clearAll}
          </button>
        )}
      </div>

      <span className="grow" />
      {/* Only once a filter is on: an unfiltered count is the list itself. */}
      {active > 0 && (
        <span className="text-muted-foreground shrink-0 text-[12px] font-[650] tabular-nums">
          {(matched === 1 ? labels.count1 : labels.count).replace('{n}', String(matched))}
        </span>
      )}
    </section>
  )
}
