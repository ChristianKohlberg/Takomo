// Behaviors: what the software must do, the tests that show it, and the runs
// that report on those tests.
//
// A behavior is written for people and may point at a section of the plan. A
// test is only a key (`playwright:editor.spec.ts › keeps edits`) linked to one
// or more behaviors; Takomo never stores test code. A run is one report — from
// CI or an agent — of outcomes per test key. A behavior's status is computed by
// the server from the latest result of each linked test; nothing sets it.
// Timestamps are ISO-8601 strings, as everywhere else in the API.
import { api } from './api'
import { fmtAge } from './format'

export const BEHAVIOR_STATUSES = ['failing', 'stale', 'untested', 'verified'] as const
export type BehaviorStatus = (typeof BEHAVIOR_STATUSES)[number]
export type Outcome = 'pass' | 'fail'

export interface LastResult {
  test: string
  outcome: Outcome
  at: string
  commit: string | null
  run: string
}

export interface Behavior {
  id: string
  project: string
  /** A plan node id, or null for a behavior about no section in particular. */
  section: string | null
  title: string
  statement: string
  /** Linked test keys, sorted. */
  tests: string[]
  status: BehaviorStatus
  last_result: LastResult | null
  created_by: string
  created_at: string
  updated_at: string
}

export interface TestResult {
  test: string
  latest: {
    outcome: Outcome
    detail: string | null
    at: string
    commit: string | null
    run: string
    actor: string
  } | null
}

export interface HistoryEntry {
  test: string
  outcome: Outcome
  detail: string | null
  at: string
  commit: string | null
  run: string
  note: string | null
  actor: string
}

export interface BehaviorDetail extends Behavior {
  test_results: TestResult[]
  /** Newest first, at most 50. */
  history: HistoryEntry[]
}

export interface StatusCounts {
  total: number
  verified: number
  failing: number
  stale: number
  untested: number
}

export interface Run {
  id: string
  project: string
  commit: string | null
  note: string | null
  actor: string
  at: string
  passed: number
  failed: number
}

export interface UnlinkedTest {
  test: string
  outcome: Outcome
  at: string
  commit: string | null
}

export interface VerificationSummary {
  fresh_days: number
  summary: StatusCounts
  /** Counts per plan node id. */
  sections: Record<string, StatusCounts>
  unsectioned: number
  unlinked_tests: { items: UnlinkedTest[]; total: number; limit: number }
  latest_run: Run | null
}

export interface Paged<T> {
  items: T[]
  total: number
  limit: number
  note?: string
}

export interface BehaviorFields {
  title: string
  statement?: string
  section?: string | null
  tests?: string[]
}

const json = { 'Content-Type': 'application/json' }
const enc = encodeURIComponent

export function listBehaviors(
  token: string,
  project: string,
  filter: { section?: string; status?: BehaviorStatus; q?: string; limit?: number; offset?: number } = {},
): Promise<Paged<Behavior>> {
  const query = new URLSearchParams()
  for (const [key, value] of Object.entries(filter)) {
    if (value !== undefined && value !== '') query.set(key, String(value))
  }
  const tail = query.toString() ? `?${query}` : ''
  return api<Paged<Behavior>>(token, `/projects/${enc(project)}/behaviors${tail}`)
}

export function fetchVerification(token: string, project: string): Promise<VerificationSummary> {
  return api<VerificationSummary>(token, `/projects/${enc(project)}/verification`)
}

export function createBehavior(token: string, project: string, fields: BehaviorFields): Promise<Behavior> {
  return api<Behavior>(token, `/projects/${enc(project)}/behaviors`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function getBehavior(token: string, id: string): Promise<BehaviorDetail> {
  return api<BehaviorDetail>(token, `/behaviors/${enc(id)}`)
}

/** `section: null` clears the link; `tests` replaces the full list. */
/** A behavior edit. `tests` replaces the whole list; prefer `add_tests` and
 * `remove_tests`, which the server applies against the current list, so a
 * link someone else makes at the same moment is kept. */
export type BehaviorPatch = Partial<BehaviorFields> & { add_tests?: string[]; remove_tests?: string[] }

export function patchBehavior(token: string, id: string, fields: BehaviorPatch): Promise<Behavior> {
  return api<Behavior>(token, `/behaviors/${enc(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function deleteBehavior(token: string, id: string): Promise<unknown> {
  return api(token, `/behaviors/${enc(id)}`, { method: 'DELETE' })
}

/** One test key per line; blank lines and surrounding whitespace dropped, duplicates removed. */
export function parseTestKeys(text: string): string[] {
  return [...new Set(text.split('\n').map((line) => line.trim()).filter(Boolean))]
}

/** "a1b2c3d · 3d" — a short commit when known, then the age. */
export function resultStamp(result: { commit: string | null; at: string }, now: number = Date.now()): string {
  const age = fmtAge(result.at, now)
  return result.commit ? `${result.commit.slice(0, 7)} · ${age}` : age
}
