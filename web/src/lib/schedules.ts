// The /v1/schedules surface, typed.
//
// A schedule creates an ORDINARY ticket on a cadence. The ticket links back to
// the schedule and to nothing else — there is no special occurrence entity to
// work, no second queue. `occurrences` below is a read-only history of the
// tickets a schedule has already produced.
//
// Response shapes were verified against a running server, not inferred: the list
// wraps in `{schedules}`, every action returns the schedule flat, and `run`
// answers `{created, schedule, ticket}`.
import { api } from './api'

export type ScheduleStatus = 'pending' | 'active' | 'paused' | 'rejected' | 'retired'
export type Unit = 'day' | 'week' | 'month'
export type Outcome = 'done' | 'not_fulfilled' | 'open'

export const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const
export const UNITS: readonly Unit[] = ['day', 'week', 'month'] as const

/**
 * How many occurrences the strip shows. Eight is two months of a weekly cadence
 * and a fortnight of a daily one — enough to see a pattern, and it fits the
 * width without shrinking the text below 11px.
 */
export const STRIP = 8

export interface Cadence {
  every: Unit
  interval?: number
  /** Weekdays, for `every: week`. */
  on?: string[]
  /** Day of month, for `every: month`. */
  day?: number
  at?: string
  tz?: string
}

export interface Template {
  title?: string
  body?: string
  labels?: string[]
  priority?: string
}

export interface Occurrence {
  slot: string
  ticket: string
  title: string
  outcome?: Outcome | null
  claimed_by?: string | null
  state?: string
  state_category?: string
  expires_at?: string | null
  archived_at?: string | null
}

export interface Schedule {
  id: string
  project: string
  name: string
  status: ScheduleStatus
  cadence?: Cadence
  cadence_error?: string | null
  template?: Template
  rationale?: string | null
  proposed_by?: string | null
  next_slot?: string | null
  upcoming?: string[]
  created_at?: string
  updated_at?: string
  /** Only present on the detail read, which is why the list is enriched per row. */
  occurrences?: Occurrence[]
}

const json = { 'Content-Type': 'application/json' }

export function listSchedules(token: string, project?: string): Promise<Schedule[]> {
  const qs = project ? `?project=${encodeURIComponent(project)}` : ''
  return api<{ schedules: Schedule[] }>(token, `/schedules${qs}`).then((b) => b.schedules ?? [])
}

export function getSchedule(token: string, id: string, occurrences = STRIP): Promise<Schedule> {
  return api<Schedule>(token, `/schedules/${encodeURIComponent(id)}?occurrences=${occurrences}`)
}

/**
 * The list route does not carry occurrence history, so each row is enriched from
 * its detail read. Bounded by the per-project cap (50), and a row that fails to
 * enrich falls back to its list shape rather than dropping off the page.
 */
export async function listWithHistory(token: string, project?: string): Promise<Schedule[]> {
  const rows = await listSchedules(token, project)
  return Promise.all(
    rows.map((s) => getSchedule(token, s.id).catch(() => s)),
  )
}

export type Action = 'activate' | 'pause' | 'resume' | 'reject'

export function scheduleAction(token: string, id: string, action: Action): Promise<Schedule> {
  return api<Schedule>(token, `/schedules/${encodeURIComponent(id)}/${action}`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

export interface RunResult {
  created: boolean
  schedule: string
  ticket: string
}

/** Fire the due slot now. Idempotent per slot: a slot that already has a ticket creates nothing. */
export function runSchedule(token: string, id: string): Promise<RunResult> {
  return api<RunResult>(token, `/schedules/${encodeURIComponent(id)}/run`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

/** Deleting a schedule keeps every ticket it already created. */
export function deleteSchedule(token: string, id: string): Promise<unknown> {
  return api<unknown>(token, `/schedules/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

export interface CreateFields {
  project: string
  name: string
  cadence: Cadence
  template: Template
  rationale?: string
}

export function createSchedule(token: string, fields: CreateFields): Promise<Schedule> {
  return api<Schedule>(token, '/schedules', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}

/** The reader's own zone, for the create form's default. */
export function guessTz(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}
