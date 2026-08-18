// The verification surface, typed: checks, cases, verdicts, environments.
//
// A check is ONE action with ONE entry precondition at ONE layer; a case is that
// check crossed with one parameter assignment, and a case is what actually gets
// executed. Takomo stores and the caller computes — nothing here generates a
// case model or judges whether a coverage claim is true.
//
// Response shapes match the server's paged envelope (`items`/`total`/`limit`,
// plus a prose `note` when the page left rows out), which is why the list
// helpers return the envelope rather than a bare array: a page a reader cannot
// tell from a complete list is how a fraction of the work reads as all of it.
import { api } from './api'

export const CHECK_LAYERS = ['ui', 'api', 'other'] as const
export const CHECK_SEVERITIES = ['blocking', 'advisory', 'low'] as const
export const ENVIRONMENT_KINDS = [
  'local',
  'ephemeral',
  'shared',
  'staging',
  'production',
  'other',
] as const
export const ENVIRONMENT_DATA_STATES = ['seeded', 'empty', 'production_like', 'unknown'] as const
export const CASE_VERDICTS = ['pass', 'fail', 'blocked', 'unreachable'] as const

export type Layer = (typeof CHECK_LAYERS)[number]
export type Severity = (typeof CHECK_SEVERITIES)[number]
export type EnvironmentKind = (typeof ENVIRONMENT_KINDS)[number]
export type Verdict = (typeof CASE_VERDICTS)[number]

/**
 * The states a case can be in, worst first.
 *
 * The order is the display order and it is deliberate: `never` and `stale` are
 * different findings and must not be merged. A case that was never run is work
 * nobody has done once; a stale one is work that was done and then invalidated
 * by a merge. Collapsing them shrinks the gap this feature exists to show.
 */
export const CASE_STATES = [
  'failed',
  'stale',
  'never',
  'blocked',
  'unreachable',
  'verified',
  'approved',
] as const
export type CaseState = (typeof CASE_STATES)[number] | 'retired'

export interface CaseCounts {
  total: number
  approved: number
  verified: number
  stale: number
  failed: number
  unreachable: number
  /** Tried and could not be judged — counted apart from `never`. */
  blocked: number
  never: number
}

export interface ResolvedPolicy {
  verification: string
  /** Which level supplied it: an inherited setting nobody can trace is worse than none. */
  verification_from: string
  expiry_days: number | null
  expiry_releases: number | null
  expiry_from: string
}

export interface CheckEnvRef {
  environment: string
  slug: string
}

export interface EnvCases extends CheckEnvRef {
  cases: CaseCounts
}

export interface CaseEnvState extends CheckEnvRef {
  state: CaseState
  agent?: { verdict: string | null; at: string | null; by: string | null }
  /**
   * `by` is the actor string the credential carried; `user` is WHICH person gave
   * it (`usr-…`), or null for a credential bound to nobody. Nothing renders the
   * person yet — this page shows where verification stands, not who signed — but
   * the shape matches the server so a display does not have to re-derive it.
   */
  human?: { verdict: string | null; at: string | null; by: string | null; user?: string | null }
  stale_since?: string | null
}

export interface Check {
  id: string
  project: string
  title: string
  body: string
  precondition: string
  layer: Layer
  severity: Severity
  epic: string | null
  /** The initiative whose conversation agreed this check should exist. */
  initiative: string | null
  globs: string[]
  /** Globs that matched NO file in the newest release — the rot, made visible. */
  orphan_globs: string[]
  /** Where this check must pass. Empty means environment-agnostic. */
  environments: CheckEnvRef[]
  /**
   * `cases` counts one row per CASE, taking the WORST of its environments, so a
   * second environment does not double the denominator. This says where the gap
   * actually is.
   */
  environment_cases: EnvCases[]
  cases: CaseCounts
  policy?: ResolvedPolicy
  cost_agent_minutes: number | null
  cost_human_minutes: number | null
  archived_at: string | null
  version: number
}

export interface CaseRow {
  id: string
  check: string
  key: string
  label: string
  state: CaseState
  /** Per-environment readings; empty when the check declares none. */
  environments: CaseEnvState[]
  assignment: unknown
  agent?: { verdict: string | null; at: string | null; by: string | null }
  /**
   * `by` is the actor string the credential carried; `user` is WHICH person gave
   * it (`usr-…`), or null for a credential bound to nobody. Nothing renders the
   * person yet — this page shows where verification stands, not who signed — but
   * the shape matches the server so a display does not have to re-derive it.
   */
  human?: { verdict: string | null; at: string | null; by: string | null; user?: string | null }
}

export interface Environment {
  id: string
  project: string
  slug: string
  name: string
  kind: EnvironmentKind
  base_url: string | null
  bring_up: string
  teardown: string
  data_state: string
  /** ADVISORY. Takomo executes nothing and cannot enforce it. */
  writable: boolean
  /** A POINTER to where a credential lives. Never a credential. */
  credentials_hint: string | null
  notes: string
  archived_at: string | null
  version: number
}

export interface WorkItem {
  check: string
  check_title: string
  environment?: string | null
  environment_slug?: string | null
  /** Where to go, carried on the item so a runner needs no second request. */
  base_url?: string | null
  severity: Severity
  layer: Layer
  case: string
  case_key: string
  case_label: string
  reason: 'stale' | 'expired' | 'failed' | 'never' | 'awaiting_human'
  verification: string
  cost_minutes: number | null
}

export interface Worklist {
  project: string
  release: { id: string; seq: number } | null
  agent: { cases: number; minutes: number | null; items: WorkItem[] }
  human: { cases: number; minutes: number | null; items: WorkItem[] }
}

export interface Gate {
  project: string
  blocked: boolean
  blocking: { agent_cases: number; human_cases: number; items: WorkItem[] }
  advisory_outstanding: number
}

export interface InitiativeVerification {
  initiative: string
  project: string
  title: string
  checks: number
  cases: CaseCounts
  percent: number
  last_verified_at: string | null
  blocked: boolean
  blocking_outstanding: number
}

export interface Paged<T> {
  items: T[]
  total: number
  limit: number
  note?: string
}

const json = { 'Content-Type': 'application/json' }
const enc = encodeURIComponent

// ---------------------------------------------------------------------------
// Checks

export function listChecks(
  token: string,
  project: string,
  opts: { initiative?: string; severity?: string; layer?: string } = {},
): Promise<Paged<Check>> {
  const qs = new URLSearchParams()
  if (opts.initiative) qs.set('initiative', opts.initiative)
  if (opts.severity) qs.set('severity', opts.severity)
  if (opts.layer) qs.set('layer', opts.layer)
  const tail = qs.toString() ? `?${qs}` : ''
  return api<Paged<Check>>(token, `/projects/${enc(project)}/checks${tail}`)
}

export interface CheckFields {
  title: string
  initiative?: string
  /** Environment ids or slugs this check must be verified in. */
  environments?: string[]
  epic?: string
  layer?: Layer
  severity?: Severity
  body?: string
  precondition?: string
  globs?: string[]
}

export function createCheck(token: string, project: string, f: CheckFields): Promise<Check> {
  return api<Check>(token, `/projects/${enc(project)}/checks`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(f),
  })
}

/** `initiative: null` detaches it — absent and null mean different things here. */
export function patchCheck(
  token: string,
  id: string,
  fields: Record<string, unknown>,
): Promise<Check> {
  return api<Check>(token, `/checks/${enc(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function archiveCheck(token: string, id: string): Promise<Check> {
  return api<Check>(token, `/checks/${enc(id)}`, { method: 'DELETE' })
}

export function listCases(token: string, check: string): Promise<Paged<CaseRow>> {
  return api<Paged<CaseRow>>(token, `/checks/${enc(check)}/cases`)
}

export interface CaseInput {
  key: string
  label?: string
  assignment?: unknown
}

/**
 * File the case set. Upsert is by `key`, so a case still present keeps its
 * verdict history, one that vanished is retired rather than deleted, and one
 * that returns is revived. `prune: false` extends instead of replacing.
 */
export function fileCases(
  token: string,
  check: string,
  cases: CaseInput[],
  prune = true,
): Promise<{ added: number; updated: number; retired: number; revived: number; live: number }> {
  return api(token, `/checks/${enc(check)}/cases`, {
    method: 'PUT',
    headers: json,
    body: JSON.stringify({ cases, ...(prune ? {} : { prune: false }) }),
  })
}

/**
 * Record what was observed.
 *
 * `actor_kind: 'human'` asserts that a PERSON approved the case and needs a
 * token carrying the `human` scope — the same line ask-a-human draws. An agent
 * records what it saw; only a person's token may claim a person approved it.
 */
export function recordVerdict(
  token: string,
  caseId: string,
  verdict: Verdict,
  opts: { note?: string; human?: boolean; environment?: string } = {},
): Promise<CaseRow> {
  const body: Record<string, unknown> = { verdict }
  if (opts.note) body.note = opts.note
  if (opts.human) body.actor_kind = 'human'
  // Required when the check declares more than one environment: the server
  // refuses rather than guessing, because filing a staging run as production is
  // worse than no record.
  if (opts.environment) body.environment = opts.environment
  return api<CaseRow>(token, `/cases/${enc(caseId)}/verdict`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(body),
  })
}

export function fetchWorklist(token: string, project: string): Promise<Worklist> {
  return api<Worklist>(token, `/projects/${enc(project)}/checklist/worklist`)
}

export function fetchGate(token: string, project: string): Promise<Gate> {
  return api<Gate>(token, `/projects/${enc(project)}/checklist/gate`)
}

export function fetchInitiativeVerification(
  token: string,
  initiative: string,
): Promise<InitiativeVerification> {
  return api<InitiativeVerification>(token, `/initiatives/${enc(initiative)}/verification`)
}

// ---------------------------------------------------------------------------
// Environments

export function listEnvironments(
  token: string,
  project: string,
  includeArchived = false,
): Promise<Paged<Environment>> {
  const tail = includeArchived ? '?archived=include' : ''
  return api<Paged<Environment>>(token, `/projects/${enc(project)}/environments${tail}`)
}

export interface EnvironmentFields {
  slug: string
  name?: string
  kind?: EnvironmentKind
  base_url?: string
  bring_up?: string
  teardown?: string
  data_state?: string
  writable?: boolean
  credentials_hint?: string
  notes?: string
}

export function createEnvironment(
  token: string,
  project: string,
  f: EnvironmentFields,
): Promise<Environment> {
  return api<Environment>(token, `/projects/${enc(project)}/environments`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(f),
  })
}

/** No `slug`: checks and tool calls address an environment by it, so it is immutable. */
export function patchEnvironment(
  token: string,
  id: string,
  fields: Record<string, unknown>,
): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function archiveEnvironment(token: string, id: string): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}`, { method: 'DELETE' })
}

export function unarchiveEnvironment(token: string, id: string): Promise<Environment> {
  return api<Environment>(token, `/environments/${enc(id)}/unarchive`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

// ---------------------------------------------------------------------------
// Derived readings the pages share

/**
 * The one state that describes a check's case set, worst first.
 *
 * Used for the badge on a collapsed row: a check with one failure and forty
 * passes is a failing check, and averaging that into a percentage is how a
 * regression hides.
 */
export function worstState(c: CaseCounts): CaseState | 'none' {
  if (!c.total) return 'none'
  if (c.failed) return 'failed'
  if (c.stale) return 'stale'
  if (c.never) return 'never'
  if (c.blocked) return 'blocked'
  if (c.approved) return 'approved'
  if (c.verified) return 'verified'
  return 'unreachable'
}

/**
 * Verified-or-approved over *verifiable* cases.
 *
 * `unreachable` is out of the denominator as well as the numerator, so a fully
 * verified project can actually reach 100% and the unreachable count stands on
 * its own as a finding rather than as a permanent deficit.
 */
export function percent(c: CaseCounts): number {
  const verifiable = c.total - c.unreachable
  if (verifiable <= 0) return 0
  return Math.round(((c.approved + c.verified) * 100) / verifiable)
}

/** A short "8 ok · 3 stale · 1 never" spread; the counts that are zero are omitted. */
export function spread(c: CaseCounts): { state: CaseState; n: number }[] {
  const out: { state: CaseState; n: number }[] = []
  const push = (state: CaseState, n: number) => {
    if (n > 0) out.push({ state, n })
  }
  push('failed', c.failed)
  push('stale', c.stale)
  push('never', c.never)
  push('blocked', c.blocked)
  push('approved', c.approved)
  push('verified', c.verified)
  push('unreachable', c.unreachable)
  return out
}
