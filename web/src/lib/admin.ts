// The admin surface: tokens, projects, and the database export.
//
// Everything here already existed as an endpoint before /settings did — the
// page is the only new part. `api()` covers the JSON calls unchanged; the one
// thing it cannot do is the export, because that response is a binary file and
// `api()` ends in `r.json()`.
import { api, API_BASE, type ApiErrorShape } from './api'

export interface TokenRow {
  id: string
  actor: string
  scopes: string[]
  /** `'*'` for an unrestricted token, else the project allowlist. */
  projects: '*' | string[]
  rate_limit: number
  created_at: string
  expires_at?: string | null
  revoked_at?: string | null
  last_used_at?: string | null
  oauth_client?: { client_id: string; client_name?: string; label?: string }
}

export interface CreatedToken extends TokenRow {
  /** The plaintext, returned ONCE at creation and never again. */
  token: string
}

export function listTokens(token: string): Promise<TokenRow[]> {
  return api<TokenRow[]>(token, '/tokens')
}

export function createToken(
  token: string,
  fields: {
    actor: string
    scopes: string[]
    projects?: string[] | null
    rate_limit?: number
    expires_seconds?: number
  },
): Promise<CreatedToken> {
  return api<CreatedToken>(token, '/tokens', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(fields),
  })
}

export function revokeToken(token: string, id: string): Promise<unknown> {
  return api(token, '/tokens/' + encodeURIComponent(id), { method: 'DELETE' })
}

export function createProject(
  token: string,
  fields: { id: string; name: string; workflow?: string },
): Promise<unknown> {
  return api(token, '/projects', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(fields),
  })
}

export function deleteProject(token: string, id: string): Promise<unknown> {
  return api(token, '/projects/' + encodeURIComponent(id), { method: 'DELETE' })
}

// ---------------------------------------------------------------------------
// The database export.

/** `attachment; filename="takomo-….sqlite"` → `takomo-….sqlite`. */
function filenameFrom(disposition: string | null, fallback: string): string {
  if (!disposition) return fallback
  const m = /filename="([^"]+)"/.exec(disposition) ?? /filename=([^;]+)/.exec(disposition)
  const name = m?.[1]?.trim()
  return name ? name : fallback
}

export interface DownloadResult {
  filename: string
  bytes: number
}

/**
 * Download the whole database as one SQLite file.
 *
 * Not `api()`: the body is a binary file, and `api()` ends in `r.json()`, which
 * would throw on the first page byte. The error path still has to read a JSON
 * error body, so it is spelled out here rather than shared.
 *
 * The response is buffered into a Blob before it is handed to the browser. A
 * bearer token cannot ride on a plain `<a download>` — the page is token-gated
 * and the server needs the Authorization header — so a fetch is the only way to
 * authenticate the request at all, and a fetch means holding the file. That is
 * fine at the sizes this store reaches and is the same trade `GET /v1/export`
 * already makes server-side; if it ever stops being fine, the fix is a
 * single-use download grant in the `tka_` spirit plus a streamed body, not a
 * change here.
 */
export async function downloadDatabase(token: string): Promise<DownloadResult> {
  const r = await fetch(API_BASE + '/export/sqlite', {
    headers: { Authorization: 'Bearer ' + token },
  })

  if (!r.ok) {
    const text = await r.text()
    let message = text
    let code: string | undefined
    try {
      const j = JSON.parse(text) as { message?: string; remedy?: string; code?: string }
      message = j.message ?? text
      code = j.code
      if (j.remedy) message += ' — ' + j.remedy
    } catch {
      // Not JSON — keep whatever the server said.
    }
    const e = new Error(message || 'HTTP ' + r.status) as ApiErrorShape
    e.status = r.status
    if (code != null) e.code = code
    // Deliberately NOT `auth: true`. A 403 here means this token may not take a
    // whole-database export — it does not mean the token is bad, and signing the
    // viewer out of a page they are otherwise entitled to would be wrong.
    throw e
  }

  const blob = await r.blob()
  const filename = filenameFrom(r.headers.get('content-disposition'), 'takomo.sqlite')

  const url = URL.createObjectURL(blob)
  try {
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    document.body.appendChild(a)
    a.click()
    a.remove()
  } finally {
    // Revoking immediately is safe: the click has already handed the blob to the
    // browser's download machinery, which holds its own reference. Skipping it
    // would pin the entire database in memory until the tab closes.
    URL.revokeObjectURL(url)
  }

  return { filename, bytes: blob.size }
}

/**
 * The token's project allowlist, or `null` when it is unrestricted.
 *
 * A function rather than an inline check because the wire format has a trap in
 * it: `/v1/whoami` reports an unrestricted token as the STRING `'*'`, and `'*'`
 * has a `.length` of 1. So the obvious `projects?.length > 0` reports "scoped to
 * one project" for the least scoped token there is, and `projects.join()` throws
 * outright. Both happened.
 */
export function projectAllowlist(who: { projects?: '*' | string[] } | null): string[] | null {
  return Array.isArray(who?.projects) && who.projects.length > 0 ? who.projects : null
}

/** Human-readable byte size for the download confirmation. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB']
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`
}
