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
