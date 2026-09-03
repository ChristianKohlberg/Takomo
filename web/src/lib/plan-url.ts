// Handing a section from one view of the plan to the other.
//
// The map and the document are two renderings of ONE tree
// (`spec/one-model-two-views.md`), so moving between them is a hand-off of a
// place rather than navigation between two features: "show it on the map" from a
// section, and "read it in the plan" from a node.
//
// Both directions are a HASH, matching `/board#t=` and `/inbox#q=`: a link
// somebody can send, and something the receiving page reads once at mount and
// then clears. It is deliberately not kept in step — a shared cursor between two
// views would drag a reader back every time the other view moved.
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
