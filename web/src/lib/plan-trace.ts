// Where a section stands, and what happened to it.
//
// The trace is the record of acts somebody would name — written, renamed, moved,
// reviewed, accepted — kept in SQL beside the plan rather than in the CRDT,
// because the CRDT update log is the mechanism that rebuilds the text and is
// rewritten wholesale by compaction. Asking it "who changed 2.1, and has anybody
// agreed with it since?" is asking a storage format a question about people.
// `src/store/trace.rs` is the other half of this.
//
// Everything here is a READING, derived on read, exactly as `mindmap-lens.ts`'s
// readings are — and `confirmed` is deliberately the word that module already
// uses for "a person has agreed with this", so the two views say one thing. What
// is new is only the source: the lens reads a stored `reviewed` flag, while this
// reads a review against the time of the last change, which is the difference
// between "somebody once looked at this" and "somebody agrees with what it says
// now".
import type { StandingRow, TraceEntry } from './mindmaps'

/**
 * How a section stands.
 *
 * Three states and no more, because this is a thing a reader glances at:
 * somebody has agreed with it as it is, somebody agreed with an older version of
 * it, or nobody has ever said anything about it.
 */
export type Standing = 'confirmed' | 'changed' | 'unseen'

/**
 * Where a section stands, from the server's grouped reading.
 *
 * `confirmed` is the server's, not recomputed here: it already knows whether the
 * review came after the last change, and recomputing it from two ISO strings in
 * the browser would be a second answer to one question. What the browser adds is
 * the distinction the boolean cannot carry — a section nobody has ever reviewed
 * reads `unseen`, not merely "not confirmed", because "nobody has looked at
 * this" and "this moved since Ada read it" call for different things.
 */
export function standingOf(row: StandingRow | null | undefined): Standing {
  if (!row || !row.reviewed_at) return 'unseen'
  return row.confirmed ? 'confirmed' : 'changed'
}

/**
 * The trace, split by section, newest first within each.
 *
 * The plan's history arrives as one page — one request rather than one per
 * section, for the same reason the map comes back whole: a view that draws every
 * section at once would otherwise open with five hundred requests. Entries that
 * belong to the plan rather than to any one section (`node: null`) are not in
 * any group; the caller shows those, if at all, against the plan itself.
 */
export function traceByNode(entries: readonly TraceEntry[]): Map<string, TraceEntry[]> {
  const out = new Map<string, TraceEntry[]>()
  for (const entry of entries) {
    if (!entry.node) continue
    const list = out.get(entry.node)
    if (list) list.push(entry)
    else out.set(entry.node, [entry])
  }
  return out
}

/**
 * Who did it, by name where there is a name.
 *
 * A person first, the credential's own actor string second. That order is the
 * point of recording `user` at all: an actor string is what a token happened to
 * be called and does not survive somebody leaving, while a user id resolves to a
 * person the directory can still name. An agent's token bound to somebody's
 * automation records that person too — "whose agent" is worth knowing.
 */
export function traceActor(entry: TraceEntry, names?: ReadonlyMap<string, string>): string {
  if (entry.user) return names?.get(entry.user) ?? entry.user
  return entry.actor
}
