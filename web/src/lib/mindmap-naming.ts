// What closing the title caret means.
//
// Naming a thought on the canvas has four endings and only one of them is a
// rename, so the decision is pulled out here rather than spread through `Live`:
// jsdom cannot lay out a canvas, and this is the part that has to be right.
//
// The load-bearing distinction is FRESH. A node the caret created has never been
// named, so abandoning it must leave nothing behind — the gesture created a box
// and a placeholder, not a thought. A node that already had a title is only ever
// restored, because nothing was written to the document while it was being typed.
//
// Emptying an EXISTING node's title is still a deletion, the way it is in every
// outliner, and it still goes through the same two questions rather than
// vanishing on the spot.

/** The node the caret is open on, as the ending depends on it. */
export interface NamingSession {
  /** Created by the gesture that opened this caret, and never named since. */
  fresh: boolean
  /** The title before the caret opened. Empty for a node that never had one. */
  previous: string
}

export type NameOutcome =
  /** Remove the node: nothing was ever created. */
  | { kind: 'discard' }
  /** Leave the document alone — the title it already has is the right one. */
  | { kind: 'keep' }
  /** An existing thought was emptied. Ask the deletion questions. */
  | { kind: 'prune' }
  | { kind: 'rename'; title: string }

/**
 * What to do when the caret closes.
 *
 * `cancelled` is Escape; anything else — Enter, Tab, or the caret losing the
 * focus — is a commit of whatever is in it.
 */
export function resolveName(
  session: NamingSession,
  input: { text: string; cancelled: boolean },
): NameOutcome {
  const title = input.text.trim()
  if (input.cancelled) return session.fresh ? { kind: 'discard' } : { kind: 'keep' }
  if (!title) return session.fresh ? { kind: 'discard' } : { kind: 'prune' }
  if (title === session.previous.trim()) return { kind: 'keep' }
  return { kind: 'rename', title }
}
