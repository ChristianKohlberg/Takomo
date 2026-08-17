// The /v1/users surface — the people directory: who work can be addressed to.
//
// A user is not a login and grants nothing on its own; scopes are still what a
// credential may do. What the directory adds is addressing — a question waiting
// on Ada rather than on whoever holds `expert:domain:billing`. See docs/users.md.
import { api } from './api'

/**
 * A person as *referenced* rather than listed — a question's `assignee`, the
 * `user` on `whoami`. Enough to render a name without a second request.
 *
 * `handle` and `name` are optional because of one deliberate server behaviour: a
 * question whose row names somebody a read did not resolve comes back carrying
 * only `{ id, label }`. That degradation exists so an unresolved person can never
 * be mistaken for an unassigned question, and `label` is always safe to render.
 */
export interface UserRef {
  id: string
  handle?: string
  name?: string
  /** What to show: the name, falling back to the handle, falling back to the id. */
  label: string
  /** The canonical `person:<handle>` tag reference for this person. */
  ref?: string
  disabled?: boolean
}

export interface User extends UserRef {
  handle: string
  email?: string | null
  meta?: Record<string, unknown>
  /**
   * Projects this person is a member of — who may be handed work where. Present
   * on the listing and the detail read; absent (not empty) on a read that did not
   * join it, so it never claims "member of nothing".
   */
  projects?: string[]
  created_at?: string
  updated_at?: string
}

export interface UsersPage {
  items: User[]
  total: number
  limit: number
  /** Prose, present only when the page left somebody out. */
  note?: string
}

export interface UserFilter {
  /** Only members of this project — the people work here can be addressed to. */
  project?: string
  /** Case-insensitive substring over handle, name and email. */
  q?: string
  includeDisabled?: boolean
  limit?: number
}

export function listUsers(token: string, f: UserFilter = {}): Promise<UsersPage> {
  const p = new URLSearchParams()
  if (f.project) p.set('project', f.project)
  if (f.q) p.set('q', f.q)
  if (f.includeDisabled) p.set('include_disabled', '1')
  if (f.limit) p.set('limit', String(f.limit))
  const qs = p.toString()
  return api<UsersPage>(token, `/users${qs ? `?${qs}` : ''}`)
}

export function getUser(token: string, handle: string): Promise<User> {
  return api<User>(token, `/users/${encodeURIComponent(handle)}`)
}

const json = { 'Content-Type': 'application/json' }

export interface NewUser {
  handle: string
  name?: string
  email?: string
  /** Memberships to create alongside the person, in one transaction. */
  projects?: string[]
}

/** Add somebody to the directory. Needs `admin`. */
export function createUser(token: string, body: NewUser): Promise<User> {
  return api<User>(token, '/users', {
    method: 'POST',
    headers: json,
    body: JSON.stringify(body),
  })
}

/**
 * Change a display name or email. Needs `admin`.
 *
 * `email: null` clears the address and an absent key leaves it alone — the two
 * mean different things on this endpoint, so this takes the field explicitly
 * rather than dropping empty strings.
 */
export function patchUser(
  token: string,
  handle: string,
  patch: { name?: string; email?: string | null },
): Promise<User> {
  return api<User>(token, `/users/${encodeURIComponent(handle)}`, {
    method: 'PATCH',
    headers: json,
    body: JSON.stringify(patch),
  })
}

/**
 * Stop new work reaching this person, or let it again. Needs `admin`.
 *
 * Neither a delete nor a revocation: every record naming them stays readable,
 * and their credentials keep working — what a credential may do is its scopes.
 */
export function setUserDisabled(token: string, handle: string, disabled: boolean): Promise<User> {
  const action = disabled ? 'disable' : 'enable'
  return api<User>(token, `/users/${encodeURIComponent(handle)}/${action}`, {
    method: 'POST',
    headers: json,
    body: '{}',
  })
}

/** Make somebody a member of a project, so work there can be addressed to them. */
export function addMembership(token: string, handle: string, project: string): Promise<unknown> {
  return api(token, `/users/${encodeURIComponent(handle)}/projects`, {
    method: 'POST',
    headers: json,
    body: JSON.stringify({ project }),
  })
}

/**
 * End a membership. Questions already addressed to them stay that way — silently
 * retracting an open decision would leave it with nobody looking at it.
 */
export function removeMembership(token: string, handle: string, project: string): Promise<unknown> {
  return api(
    token,
    `/users/${encodeURIComponent(handle)}/projects/${encodeURIComponent(project)}`,
    { method: 'DELETE' },
  )
}
