// Handing a section from one view of the plan to the other.
//
// The map and the document are two renderings of ONE tree
// (`spec/one-model-two-views.md`), so moving between them is a hand-off of a
// place rather than navigation between two features: "show it on the map" from a
// section, and "read it in the plan" from a node.
//
// A section is kept in `#n=` across all three views. The workspace router adds
// `?project=` so a bookmark and browser Back restore both scope and section.
// Selection is personal navigation state, never a shared CRDT cursor.
//
// Pure and string-only, so both directions are testable without a router, a
// canvas or a socket.

const enc = encodeURIComponent

/** Where the map sends somebody who wants this section written out. */
export function planLink(node?: string | null): string {
  return node ? `/documents#n=${enc(node)}` : '/documents'
}

/** Where the document sends somebody who wants this section on the canvas. The
 *  map is named explicitly, because `/mindmaps` opens whichever map it is given
 *  rather than the stored project's. */
export function mapLink(map: string, node?: string | null): string {
  return node ? `/mindmaps#m=${enc(map)}&n=${enc(node)}` : `/mindmaps#m=${enc(map)}`
}

/**
 * Where either view sends somebody who wants this section's tests.
 *
 * The same `n=` hash the other two links use, so `readPlanFocus` reads all
 * three — a check names a node id, and `/verification` resolves it against the
 * project's plan rather than being handed a map as well.
 */
export function testsLink(node?: string | null): string {
  return node ? `/verification#n=${enc(node)}` : '/verification'
}

/**
 * The section a `/documents` URL is asking for, or null.
 *
 * Takes the raw `location.hash`, leading `#` and all — that is what a caller
 * has — and equally a whole link, so what `planLink` writes can be read straight
 * back. An unparseable hash is no ask rather than an error: a link somebody
 * mangled should open the plan, not fail to open it.
 */
export function readPlanFocus(hash: string): string | null {
  const at = hash.indexOf('#')
  const raw = at >= 0 ? hash.slice(at + 1) : hash
  const node = new URLSearchParams(raw).get('n')
  return node && node.trim() ? node : null
}
