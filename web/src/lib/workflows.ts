// Workflows: a project's own state machine, the shared library, and the
// server-side validation the editor leans on.
//
// The types here are precise where `lib/board.ts`'s are loose. The board only
// needs a workflow's states to draw its columns, so `transitions?: unknown` is
// honest there; an editor has to round-trip the whole document byte for byte,
// and a field it does not model is a field it would silently delete on save.
import { api } from './api'

const json = { 'Content-Type': 'application/json' }

/** The six fixed categories generic tooling reasons about. */
export const CATEGORIES = [
  'todo',
  'in_progress',
  'blocked',
  'review',
  'done',
  'cancelled',
] as const
export type Category = (typeof CATEGORIES)[number]

export interface WfState {
  id: string
  category: Category
  claimable?: boolean
  terminal?: boolean
}

export interface WfTransition {
  from: string
  to: string
  requires?: string[]
}

export interface WorkflowDoc {
  name: string
  initial: string
  states: WfState[]
  transitions: WfTransition[]
  /**
   * Guard descriptions, keyed by guard id. Carried through untouched: the
   * editor does not author them, and dropping a key it does not understand
   * would quietly rewrite the project's workflow on the next save.
   */
  guards?: unknown
}

/** Node positions for the canvas, keyed by state id. */
export type Layout = Record<string, { x: number; y: number }>

export interface WorkflowEntry {
  id: string
  name: string
  description?: string | null
  workflow: WorkflowDoc
  layout?: Layout | null
  /** Ships with the server; reseeded on every start, so not editable. */
  builtin: boolean
  created_at: string
  created_by: string
  updated_at: string
}

// ---------------------------------------------------------------------------
// A project's own workflow.

export function getProjectWorkflow(token: string, project: string): Promise<WorkflowDoc> {
  return api<WorkflowDoc>(token, `/projects/${encodeURIComponent(project)}/workflow`)
}

export function putProjectWorkflow(
  token: string,
  project: string,
  wf: WorkflowDoc,
): Promise<WorkflowDoc> {
  return api<WorkflowDoc>(token, `/projects/${encodeURIComponent(project)}/workflow`, {
    method: 'PUT',
    headers: json,
    body: JSON.stringify(wf),
  })
}

export interface Verdict {
  valid: boolean
  problems: string[]
}

/**
 * Would this document be accepted, without writing it?
 *
 * The editor validates against the SERVER rather than reimplementing the rules.
 * They are subtle — terminal reachability by reverse BFS, the
 * claimable-with-done-category trap — and one of them cannot be computed here at
 * all: whether a ticket currently sits in a state the draft drops depends on the
 * project's live data. A second copy of those rules in TypeScript would drift,
 * and the drift would show up as "the editor said it was fine and Apply 422'd".
 */
export function validateWorkflow(
  token: string,
  project: string,
  wf: WorkflowDoc,
): Promise<Verdict> {
  return api<Verdict>(token, `/projects/${encodeURIComponent(project)}/workflow/validate`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify(wf),
  })
}

// ---------------------------------------------------------------------------
// Canvas layout. Stored beside the workflow, never inside it: the document is
// `deny_unknown_fields` server-side, so a positions key would 422 on save.

export function getLayout(token: string, project: string): Promise<{ layout: Layout | null }> {
  return api<{ layout: Layout | null }>(
    token,
    `/projects/${encodeURIComponent(project)}/workflow-layout`,
  )
}

export function putLayout(token: string, project: string, layout: Layout): Promise<unknown> {
  return api(token, `/projects/${encodeURIComponent(project)}/workflow-layout`, {
    method: 'PUT',
    headers: json,
    body: JSON.stringify({ layout }),
  })
}

// ---------------------------------------------------------------------------
// Preflight: which states actually hold tickets right now.

interface MetricsResponse {
  projects?: Record<string, { by_state?: Record<string, number> }>
}

/**
 * Ticket counts per state for one project.
 *
 * Read off `/v1/metrics`, which already reports exactly this — no endpoint was
 * added for the preflight. The counts are what turn "this change is refused"
 * into "these four tickets are in `implementing`, move them first".
 */
export async function getStateCounts(
  token: string,
  project: string,
): Promise<Record<string, number>> {
  const m = await api<MetricsResponse>(token, '/metrics')
  return m.projects?.[project]?.by_state ?? {}
}

/**
 * States that hold tickets but are missing from the draft — the exact reason the
 * server would refuse to apply it.
 *
 * Computed here as well as server-side on purpose, and it is not a duplicated
 * RULE: the server owns the verdict, this owns the EXPLANATION. The 422 names
 * the states in a sentence; the reader needs them as rows, with counts, before
 * pressing anything.
 */
export function strandedStates(
  wf: WorkflowDoc,
  counts: Record<string, number>,
): { state: string; tickets: number }[] {
  const defined = new Set(wf.states.map((s) => s.id))
  return Object.entries(counts)
    .filter(([state, n]) => n > 0 && !defined.has(state))
    .map(([state, tickets]) => ({ state, tickets }))
    .sort((a, b) => b.tickets - a.tickets || a.state.localeCompare(b.state))
}

// ---------------------------------------------------------------------------
// The library.

export function listWorkflows(token: string): Promise<WorkflowEntry[]> {
  return api<WorkflowEntry[]>(token, '/workflows')
}

export function createWorkflowEntry(
  token: string,
  fields: { name: string; description?: string; workflow: WorkflowDoc; layout?: Layout },
): Promise<WorkflowEntry> {
  return api<WorkflowEntry>(token, '/workflows', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function patchWorkflowEntry(
  token: string,
  id: string,
  fields: { name?: string; description?: string | null; workflow?: WorkflowDoc; layout?: Layout },
): Promise<WorkflowEntry> {
  return api<WorkflowEntry>(token, `/workflows/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(fields),
  })
}

export function deleteWorkflowEntry(token: string, id: string): Promise<unknown> {
  return api(token, `/workflows/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

// ---------------------------------------------------------------------------
// Requirements on a transition.

export const REQ_CLAIM = 'claim'
export const REQ_HUMAN = 'scope:human'
export const REQ_NO_CHILDREN = 'guard:no_open_children'
export const REQ_NO_BLOCKERS = 'guard:no_open_blockers'
export const HAS_LINK = 'guard:has_link:'

/** The link key a `guard:has_link:<key>` requirement demands, if present. */
export function hasLinkKey(requires: string[] | undefined): string | null {
  const found = (requires ?? []).find((r) => r.startsWith(HAS_LINK))
  return found ? found.slice(HAS_LINK.length) : null
}

/** Add or remove one requirement, preserving the order of the rest. */
export function toggleRequirement(
  requires: string[] | undefined,
  req: string,
  on: boolean,
): string[] {
  const cur = requires ?? []
  if (on) return cur.includes(req) ? [...cur] : [...cur, req]
  return cur.filter((r) => r !== req)
}

/** Replace the has_link key; an empty key removes the requirement entirely. */
export function setHasLink(requires: string[] | undefined, key: string): string[] {
  const without = (requires ?? []).filter((r) => !r.startsWith(HAS_LINK))
  const trimmed = key.trim()
  return trimmed ? [...without, HAS_LINK + trimmed] : without
}
