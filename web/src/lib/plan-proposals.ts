// What an agent has offered against the plan, read section by section.
//
// Proposals live in the MAP's Y.Doc, in a top-level `proposals` map, exactly as
// they did in a standalone document's — the record is JSON in a `Y.Map` entry
// and `lib/doc-ops.ts` parses it. What is new is one field: `node`, the section
// the proposal is about. So one map serves the whole plan and this module is
// what turns it into "what is waiting on section 2.1".
//
// It is read from the CRDT rather than from `GET /v1/mindmaps/{id}/proposals`,
// and that is the same decision the standalone editor made for the same reason:
// the page already holds the document, so a proposal an agent writes appears in
// an open browser at once and survives a disconnect. The REST route is for
// readers that are not holding the document open.
//
// Yjs is imported as a TYPE only. Nothing here constructs a shared type — it
// reads entries and writes one back — so this module costs nothing at runtime
// beyond its own code.
import type * as Y from 'yjs'

import { parseProposal, type Proposal } from './doc-ops'
import type { PlanSection } from './plan-sections'

/** The top-level key. Mirrors `PROPOSALS_FIELD` in `src/api/docprops.rs`. */
export const PROPOSALS_KEY = 'proposals'

export type ProposalMap = Y.Map<string>

/** Every proposal in the document, unreadable entries dropped rather than
 *  thrown on: one malformed record must not blank the panel. */
export function readProposals(map: ProposalMap): Proposal[] {
  const out: Proposal[] = []
  map.forEach((raw) => {
    const p = parseProposal(raw)
    if (p) out.push(p)
  })
  return out
}

/**
 * Proposals grouped by the section they are about.
 *
 * A record with no `node` belongs to no section — that is what a standalone
 * document's proposals look like, and one arriving here would be about a
 * document this view is not showing. It is left out of every group rather than
 * being attached to an arbitrary section.
 */
export function proposalsByNode(list: readonly Proposal[]): Map<string, Proposal[]> {
  const out = new Map<string, Proposal[]>()
  for (const p of list) {
    if (!p.node) continue
    const group = out.get(p.node)
    if (group) group.push(p)
    else out.set(p.node, [p])
  }
  for (const group of out.values()) group.sort(byUndecidedThenNewest)
  return out
}

/**
 * Pending first, newest first within that.
 *
 * A decided proposal is a record and a pending one is work, so the work comes
 * first — and neither disappears, because "we considered this and said no" is
 * the thing somebody wants three weeks later when it is proposed again.
 */
export function byUndecidedThenNewest(a: Proposal, b: Proposal): number {
  if (a.status !== b.status) {
    if (a.status === 'pending') return -1
    if (b.status === 'pending') return 1
  }
  return b.created_at - a.created_at
}

/** How many proposals are waiting on a person, per section. */
export function pendingByNode(list: readonly Proposal[]): Record<string, number> {
  const out: Record<string, number> = {}
  for (const p of list) {
    if (!p.node || p.status !== 'pending') continue
    out[p.node] = (out[p.node] ?? 0) + 1
  }
  return out
}

/**
 * Everything waiting beneath this section, itself included.
 *
 * What a FOLDED row reports. A reader who has folded a branch has not decided
 * they do not care what is in it, and a proposal nobody can see is the failure
 * this whole panel exists to prevent.
 */
export function pendingInSubtree(
  section: PlanSection,
  pending: Readonly<Record<string, number>>,
): number {
  let total = pending[section.key] ?? 0
  for (const child of section.children) total += pendingInSubtree(child, pending)
  return total
}

/**
 * The blocks a section's pending proposals touch, as one stable string.
 *
 * A string rather than a `Set`, deliberately. The editor highlights these
 * through a ProseMirror transaction, so the effect that dispatches it must fire
 * when the SET changes and not merely when the page re-rendered — and a fresh
 * `Set` is a new identity every time. Sorted and joined, it compares by value.
 */
export function highlightKeyFor(list: readonly Proposal[]): string {
  const ids = new Set<string>()
  for (const p of list) {
    if (p.status !== 'pending') continue
    for (const op of p.ops) ids.add(op.id)
  }
  return [...ids].sort().join(' ')
}

/**
 * Record a decision on the proposal itself.
 *
 * The record is UPDATED, never removed: a rejected proposal stays visible as
 * rejected, because it is a signal about the plan somebody was wrong about.
 *
 * Returns false when the proposal is already decided in THIS replica — which
 * catches a double click, and a second reviewer whose peer has already seen the
 * first decision.
 *
 * It is NOT consensus, and the comment here used to say it was. The map is a
 * CRDT with last-write-wins per key, so two peers that have not yet seen each
 * other both read `pending`, both return true, and both apply — measured, with
 * the paragraph inserted twice and one `decided_by` surviving. Closing that
 * needs the decision to converge before the ops are applied, which cannot be
 * done offline; the honest position is that this narrows the window rather than
 * removing it. `dropped` exists because of it: what an accept could not apply is
 * written onto the record, so a reader afterwards can see the difference between
 * "accepted" and "accepted and landed whole".
 */
export function decideProposal(
  map: ProposalMap,
  id: string,
  status: 'accepted' | 'rejected',
  by: string,
  now: number = Date.now(),
  dropped: readonly string[] = [],
): boolean {
  const current = parseProposal(map.get(id))
  if (!current || current.status !== 'pending') return false
  map.set(
    id,
    JSON.stringify({
      ...current,
      status,
      decided_by: by,
      decided_at: now,
      ...(dropped.length ? { dropped: [...dropped] } : {}),
    }),
  )
  return true
}
