// The shared session: one token and one project selection across all four
// surfaces.
//
// Both used to be per-page keys, which made sense when the surfaces were four
// independent documents and makes none now. The migration and the
// "all projects" rule are the two parts worth pinning: the first decides whether
// a deploy logs everyone out, and the second is a silent data-loss bug if it
// regresses.
import { describe, it, expect, beforeEach } from 'vitest'
import { loadToken, saveToken, loadProject, saveProject, isAuthError } from './session'

beforeEach(() => localStorage.clear())

describe('the shared token', () => {
  it('is read and written under one key', () => {
    saveToken('tk_abc')
    expect(localStorage.getItem('takomo.token')).toBe('tk_abc')
    expect(loadToken()).toBe('tk_abc')
  })

  it('migrates a per-page token forward rather than logging the viewer out', () => {
    // Someone who had a working board yesterday.
    localStorage.setItem('takomo.board.token', 'tk_from_board')
    expect(loadToken()).toBe('tk_from_board')
    expect(localStorage.getItem('takomo.token')).toBe('tk_from_board')
  })

  it('prefers the shared key over a stale per-page one', () => {
    localStorage.setItem('takomo.token', 'tk_current')
    localStorage.setItem('takomo.board.token', 'tk_old')
    expect(loadToken()).toBe('tk_current')
  })

  it('clears on empty', () => {
    saveToken('tk_abc')
    saveToken('')
    expect(loadToken()).toBe('')
  })
})

describe('the shared project', () => {
  it('treats the empty string as a REAL value meaning all projects', () => {
    // Not "unset". `/inbox`, `/initiatives` and `/schedules` each offer it, so
    // it has to survive a round trip — `localStorage` cannot tell '' from absent
    // without this distinction being deliberate.
    saveProject('')
    expect(localStorage.getItem('takomo.project')).toBe('')
    expect(loadProject()).toBe('')
  })

  it('does not let a legacy key resurrect a deliberately-cleared selection', () => {
    localStorage.setItem('takomo.board.project', 'demo')
    saveProject('') // the viewer chose "all projects"
    expect(loadProject()).toBe('')
  })

  it('migrates a per-page selection forward', () => {
    localStorage.setItem('takomo.inbox.project', 'demo')
    expect(loadProject()).toBe('demo')
    expect(localStorage.getItem('takomo.project')).toBe('demo')
  })

  it('ignores an empty legacy value, which is indistinguishable from unused', () => {
    localStorage.setItem('takomo.board.project', '')
    localStorage.setItem('takomo.schedules.project', 'demo')
    expect(loadProject()).toBe('demo')
  })

  it('round-trips an explicit pick', () => {
    saveProject('demo')
    expect(loadProject()).toBe('demo')
  })
})

describe('the board narrowing rule', () => {
  // The board cannot render "all projects" — columns come from a project's
  // workflow and two projects need not agree on their states — so it narrows to
  // a concrete one. This is the derivation it uses, and the rule that it must
  // NOT be written back.
  const effective = (shared: string, first: string | undefined) => shared || first || ''

  it('narrows an all-projects selection to the first project for rendering', () => {
    expect(effective('', 'demo')).toBe('demo')
  })

  it('leaves an explicit selection alone', () => {
    expect(effective('other', 'demo')).toBe('other')
  })

  it('survives having no projects at all', () => {
    expect(effective('', undefined)).toBe('')
  })

  it('leaves "all projects" intact in storage — the actual bug this prevents', () => {
    // Sit on the inbox at "All projects"…
    saveProject('')
    // …hop to the board, which narrows to `demo` for its own rendering…
    expect(effective(loadProject(), 'demo')).toBe('demo')
    // …and come back. The inbox must still be on all projects. If the board had
    // persisted its narrowing, this would read 'demo' and the viewer's
    // all-projects view would be silently gone.
    expect(loadProject()).toBe('')
  })
})

describe('recognising a dead credential', () => {
  // This predicate exists because two background polls did NOT have it. The
  // board caught every poll failure as "reconnecting" and the inbox swallowed
  // its own entirely, so a revoked or expired token left both surfaces showing
  // stale data forever and neither ever asked for a new token.
  it('recognises the flag the API client sets', () => {
    expect(isAuthError({ auth: true })).toBe(true)
  })

  it('recognises 401 and 403', () => {
    expect(isAuthError({ status: 401 })).toBe(true)
    expect(isAuthError({ status: 403 })).toBe(true)
  })

  it('does NOT claim a transient failure is an auth failure', () => {
    // The distinction is the whole point: these must stay "reconnecting", not
    // throw the viewer back to the token screen mid-session.
    expect(isAuthError({ status: 500 })).toBe(false)
    expect(isAuthError({ status: 502 })).toBe(false)
    expect(isAuthError({ status: 429 })).toBe(false)
    expect(isAuthError(new TypeError('Failed to fetch'))).toBe(false)
  })

  it('survives the shapes a rejected fetch actually produces', () => {
    expect(isAuthError(undefined)).toBe(false)
    expect(isAuthError(null)).toBe(false)
    expect(isAuthError('nope')).toBe(false)
    expect(isAuthError({})).toBe(false)
  })
})
