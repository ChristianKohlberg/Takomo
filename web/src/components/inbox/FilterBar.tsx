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
//
// Every filter here answers a question a triaging human actually asks: which
// ticket, which words, how urgent, is it blocking work, is it mine, is it about
// to auto-resolve, and is it even waiting on me. The last one matters most: a
// question bounced back to its agent (`awaiting: agent`) is not the reader's to
// answer, and it sat in Open indistinguishable from one that was.
import { Typeahead, type TypeaheadOption } from '../Typeahead'
import { URGENCIES } from '@/lib/question-filters'
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
  /** Chips and toggles. */
  urgency: string
  critical: string
  high: string
  normal: string
  low: string
  allModes: string
  blocking: string
  advisory: string
  mine: string
  mineHint: string
  waiting: string
  waitingHint: string
  soon: string
  soonHint: string
  allAskers: string
  asker: string
  /** Assignee picker: who a question is waiting on. */
  anyAssignee: string
  assignee: string
  unassigned: string
  groupEpic: string
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
  urgency: string[]
  onUrgency: (levels: string[]) => void
  mode: 'blocking' | 'advisory' | ''
  onMode: (mode: 'blocking' | 'advisory' | '') => void
  /** Undefined when the reader holds no `expert:` scope — the toggle is hidden. */
  mine?: boolean
  onMine?: (on: boolean) => void
  hideAwaitingAgent: boolean
  onHideAwaitingAgent: (on: boolean) => void
  expiringSoon: boolean
  onExpiringSoon: (on: boolean) => void
  askers: string[]
  askedBy: string
  onAskedBy: (actor: string) => void
  /**
   * People a question in this view is waiting on, by handle. Empty = the control
   * is hidden: on an instance with no directory it could only ever narrow to
   * nothing.
   */
  assignees: { handle: string; label: string }[]
  assignee: string
  onAssignee: (handle: string) => void
  group: boolean
  onGroup: (on: boolean) => void
  /** How many questions survive the filters, across every folder. */
  matched: number
  activeCount: number
  onClear: () => void
  /** Whether the phone's collapsed bank is expanded. */
  open: boolean
  onOpen: (open: boolean) => void
  labels: FilterBarLabels
  className?: string
}

const chip =
  'shrink-0 cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12.5px] font-[650] transition-colors'

export function FilterBar({
  tickets,
  ticket,
  onTicket,
  search,
  onSearch,
  urgency,
  onUrgency,
  mode,
  onMode,
  mine,
  onMine,
  hideAwaitingAgent,
  onHideAwaitingAgent,
  expiringSoon,
  onExpiringSoon,
  askers,
  askedBy,
  onAskedBy,
  assignees,
  assignee,
  onAssignee,
  group,
  onGroup,
  matched,
  activeCount,
  onClear,
  open,
  onOpen,
  labels,
  className,
}: FilterBarProps) {
  const urgencyLabel: Record<string, string> = {
    critical: labels.critical,
    high: labels.high,
    normal: labels.normal,
    low: labels.low,
  }

  const toggleUrgency = (level: string) =>
    onUrgency(urgency.includes(level) ? urgency.filter((u) => u !== level) : [...urgency, level])

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
        {activeCount > 0 && (
          <span className="bg-primary text-primary-foreground ml-1.5 inline-block min-w-[17px] rounded-[9px] px-1.25 text-center text-[11px] font-bold">
            {activeCount}
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

        {/* Urgency is the one thing already visible on every row (the coloured
            left rule) that could not be filtered on. Multi-select, because
            "critical and high" is the actual triage question, not "critical". */}
        <div role="group" aria-label={labels.urgency} className="flex flex-wrap items-center gap-1.5">
          {URGENCIES.map((level) => {
            const on = urgency.includes(level)
            return (
              <button
                key={level}
                type="button"
                aria-pressed={on}
                onClick={() => toggleUrgency(level)}
                className={cn(
                  chip,
                  on
                    ? 'border-primary bg-secondary text-primary'
                    : 'border-border text-muted-foreground',
                )}
              >
                {urgencyLabel[level] ?? level}
              </button>
            )
          })}
        </div>

        <select
          aria-label={labels.allModes}
          value={mode}
          onChange={(e) => onMode(e.target.value as 'blocking' | 'advisory' | '')}
          className="bg-muted text-foreground border-border cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
        >
          <option value="">{labels.allModes}</option>
          <option value="blocking">{labels.blocking}</option>
          <option value="advisory">{labels.advisory}</option>
        </select>

        {askers.length > 1 && (
          <select
            aria-label={labels.asker}
            value={askedBy}
            onChange={(e) => onAskedBy(e.target.value)}
            className="bg-muted text-foreground border-border max-w-45 cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
          >
            <option value="">{labels.allAskers}</option>
            {askers.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        )}

        {/* Who it is waiting on. `none` earns its place beside the people: the
            open queue is the one bucket a triaging reader most needs to find, and
            it is invisible in a list where most rows carry no name either. */}
        {assignees.length > 0 && (
          <select
            aria-label={labels.assignee}
            value={assignee}
            onChange={(e) => onAssignee(e.target.value)}
            className="bg-muted text-foreground border-border max-w-45 cursor-pointer rounded-lg border px-2.5 py-1.5 text-[13px] font-[650]"
          >
            <option value="">{labels.anyAssignee}</option>
            <option value="none">{labels.unassigned}</option>
            {assignees.map((p) => (
              <option key={p.handle} value={p.handle}>
                {p.label}
              </option>
            ))}
          </select>
        )}

        {/* Only for a reader the board can route to: "for me" is meaningless
            without either an `expert:` scope or a person behind the credential,
            and a toggle that can only ever empty the list is worse than none. */}
        {mine !== undefined && onMine && (
          <label
            title={labels.mineHint}
            className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]"
          >
            <input
              type="checkbox"
              className="size-4"
              checked={mine}
              onChange={(e) => onMine(e.target.checked)}
            />
            {labels.mine}
          </label>
        )}

        <label
          title={labels.waitingHint}
          className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]"
        >
          <input
            type="checkbox"
            className="size-4"
            checked={hideAwaitingAgent}
            onChange={(e) => onHideAwaitingAgent(e.target.checked)}
          />
          {labels.waiting}
        </label>

        <label
          title={labels.soonHint}
          className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]"
        >
          <input
            type="checkbox"
            className="size-4"
            checked={expiringSoon}
            onChange={(e) => onExpiringSoon(e.target.checked)}
          />
          {labels.soon}
        </label>

        {/* Grouping is not a filter — it hides nothing — so it sits outside the
            active count and survives "clear filters". */}
        <label className="text-muted-foreground flex cursor-pointer items-center gap-1.5 py-2 text-[12px] font-[650]">
          <input
            type="checkbox"
            className="size-4"
            checked={group}
            onChange={(e) => onGroup(e.target.checked)}
          />
          {labels.groupEpic}
        </label>

        {activeCount > 0 && (
          <button
            type="button"
            onClick={onClear}
            className="text-primary cursor-pointer px-2 py-1.5 text-[13px] font-[650] underline"
          >
            {labels.clearAll}
          </button>
        )}
      </div>

      <span className="grow" />
      {/* Only once a filter is on: an unfiltered count is the list itself. */}
      {activeCount > 0 && (
        <span className="text-muted-foreground shrink-0 text-[12px] font-[650] tabular-nums">
          {(matched === 1 ? labels.count1 : labels.count).replace('{n}', String(matched))}
        </span>
      )}
    </section>
  )
}
