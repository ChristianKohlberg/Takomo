// The one HTTP client. Ported from the four forked `api()` helpers the pages
// carried (board's was 28 lines, inbox's 17, and they had drifted) — that fork
// is the reason this module exists.
//
// The error path is the important part and it is not generic: Takomo's error
// bodies are flat (`{code, message, remedy}`) and the message is written for a
// reader who must act on it. Surfacing `message` + `remedy` verbatim is what
// turns a failure into something the user can fix; re-wording it here would
// throw away the most carefully built thing in the API.
export interface ApiErrorShape extends Error {
  status?: number
  /** 401 — the caller shows the token gate rather than an error toast. */
  auth?: boolean
  code?: string
}

export interface ApiOptions {
  method?: string
  body?: BodyInit | null
  headers?: Record<string, string>
  signal?: AbortSignal
}

/** Same-origin by default: the binary serves both the page and `/v1`. */
export const API_BASE = '/v1'

function apiError(message: string, status?: number, code?: string): ApiErrorShape {
  const e = new Error(message) as ApiErrorShape
  if (status != null) e.status = status
  if (code != null) e.code = code
  return e
}

export async function api<T = unknown>(
  token: string,
  path: string,
  opts: ApiOptions = {},
): Promise<T> {
  const headers: Record<string, string> = { ...(opts.headers ?? {}) }
  headers['Authorization'] = 'Bearer ' + token

  const init: RequestInit = { headers, method: opts.method ?? 'GET' }
  if (opts.body != null) init.body = opts.body
  if (opts.signal) init.signal = opts.signal

  const r = await fetch(API_BASE + path, init)

  // 401 is the ONLY status that means "this credential is not usable" — the
  // token is missing, malformed, revoked or expired, and the gate is the one
  // thing that helps. Every 403 the server emits is the opposite: an authentic
  // token refused ONE operation, with a stable `code` and a message written to
  // say what scope is missing and how to get it (`auth.scope`, `auth.project`,
  // `question.approve_expertise`, the transition guards, the export). Treating
  // those as an auth failure threw that message away and signed the reader out
  // of a console they may legitimately use — so a `human` token meeting an
  // expert-gated approve lost its session instead of being told why. 403 falls
  // through to the normal error path and reaches the user as a toast.
  if (r.status === 401) {
    const e = apiError('auth', r.status)
    e.auth = true
    throw e
  }

  if (!r.ok) {
    const text = await r.text()
    let message = text
    let code: string | undefined
    try {
      const j = JSON.parse(text) as { message?: string; remedy?: string; code?: string; error?: { message?: string } }
      message = j.message ?? j.error?.message ?? text
      code = j.code
      // The remedy says what to DO about it. Dropping it is how a teaching
      // error becomes a wall.
      if (j.remedy) message += ' — ' + j.remedy
    } catch {
      // Not JSON (a proxy error page, an empty body): keep the raw text.
    }
    throw apiError(message || 'HTTP ' + r.status, r.status, code)
  }

  // A successful response with no body is not an error, and this used to treat
  // it as one: `r.json()` on a 204 throws "Unexpected end of JSON input".
  //
  // Every DELETE in the API answers 204 — tokens, projects, shares, answer
  // links, deps — so the failure mode was specific and nasty. The request had
  // ALREADY SUCCEEDED by the time the parse blew up, so the caller's catch ran
  // over a completed mutation: the UI reported "Request failed", skipped its
  // refetch, and left the now-revoked token on screen looking live. Being told a
  // revoke failed when it succeeded is the wrong direction to be wrong in.
  //
  // Read the body as text first and only parse when there is something to parse.
  // `T` is what the caller declared; a void endpoint is declared `unknown`.
  const text = await r.text()
  if (!text) return undefined as T
  return JSON.parse(text) as T
}
