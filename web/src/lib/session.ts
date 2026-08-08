// The viewer's token, shared by every surface.
//
// Each page used to keep its own key — `takomo.board.token`,
// `takomo.inbox.token`, and two more — so opening the inbox after the board
// meant pasting the same token a second time. That was defensible when the four
// pages were four independent documents. With one app it is just a bug, so
// there is now ONE key.
//
// The old keys are still read, once, as a fallback: someone who had a working
// board yesterday should not be logged out by a deploy. The first successful
// read migrates the value forward and the old keys are left alone (harmless,
// and it keeps the previous build working if anyone rolls back).
const KEY = 'takomo.token'

/** The per-surface keys this replaced, newest-used first. */
const LEGACY = [
  'takomo.board.token',
  'takomo.inbox.token',
  'takomo.initiatives.token',
  'takomo.schedules.token',
]

export function loadToken(): string {
  const current = localStorage.getItem(KEY)
  if (current) return current
  for (const k of LEGACY) {
    const v = localStorage.getItem(k)
    if (v) {
      localStorage.setItem(KEY, v)
      return v
    }
  }
  return ''
}

export function saveToken(token: string): void {
  if (token) localStorage.setItem(KEY, token)
  else localStorage.removeItem(KEY)
}

// ---------------------------------------------------------------------------

/**
 * The selected project, shared by every surface.
 *
 * Same story as the token: four keys made sense for four independent documents
 * and make none for one app. Picking `demo` on the board and finding the inbox
 * still on something else is the kind of seam that tells you the surfaces are
 * not really one product.
 *
 * `''` means ALL PROJECTS, and that is a real value, not "unset". `/inbox`,
 * `/initiatives` and `/schedules` each offer it explicitly. `/board` cannot show
 * it — a kanban across projects would have to reconcile per-project workflow
 * states — so the board narrows to a concrete project for its own rendering.
 *
 * What the board must NOT do is write that narrowing back here. If it did,
 * sitting on the inbox at "All projects", hopping to the board and coming back
 * would silently have lost the all-projects view. The board's constraint is the
 * board's; only an explicit pick by a human changes the shared selection.
 */
const PROJECT_KEY = 'takomo.project'

const LEGACY_PROJECT = [
  'takomo.board.project',
  'takomo.inbox.project',
  'takomo.initiatives.project',
  'takomo.schedules.project',
]

export function loadProject(): string {
  const current = localStorage.getItem(PROJECT_KEY)
  if (current !== null) return current
  for (const k of LEGACY_PROJECT) {
    const v = localStorage.getItem(k)
    // Only a non-empty legacy value is worth carrying over: an empty one is
    // indistinguishable from "this page was never used".
    if (v) {
      localStorage.setItem(PROJECT_KEY, v)
      return v
    }
  }
  return ''
}

/** Records an EXPLICIT choice. `''` is stored, because it means "all projects". */
export function saveProject(id: string): void {
  localStorage.setItem(PROJECT_KEY, id)
}
