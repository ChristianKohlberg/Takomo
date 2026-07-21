# Token scheme

Basic authentication done properly-but-simply: bearer tokens, scoped, hashed at rest, minted by CLI. No users, no OAuth, no sessions in v1.

## Token format

```
tk_<22 chars base62>            # ~128 bits of randomness
```

- Sent as `Authorization: Bearer tk_...` on every request (only `/healthz` is open).
- Stored server-side as SHA-256 hash only; the plaintext is shown once at mint time.
- The token row carries: `actor` (display name), `scopes`, `projects` (list or `*`), `created_at`, `expires_at` (optional), `revoked_at`, `last_used_at`, `rate_limit` (per-minute write budget, default 120).

## Actor attribution

`actor` is the identity everywhere: `created_by`, comment authors, event `actor`, claim `holder`. One token per agent/orchestrator/human; never share tokens across actors, or the audit trail and lease forensics lose meaning. Cheap to mint, cheap to revoke.

Naming convention: `human:alice`, `orch:main`, `agent:runner-1`, `svc:github-webhook`.

## Scopes

| scope     | grants |
|-----------|--------|
| `read`    | all GETs (including `GET /v1/export`, `GET /v1/metrics`, and `GET /v1/projects/{project}/roadmap`, each scoped to the token's readable projects) |
| `write`   | create/patch/comment/deps/claim/heartbeat/release/transition/archive/unarchive (subject to workflow `requires`) |
| `human`   | satisfies `scope:human` transition requirements (approval gates) |
| `autoland`| satisfies `scope:autoland` (or other custom scopes a workflow names — scopes beyond the four reserved ones are free-form strings matched literally) |
| `admin`   | projects (create **and delete**), workflow upload, token management |

Typical grants: workers get `read,write` on their project; orchestrators get `read,write` plus `autoland` where yolo applies; humans get `read,write,human`; the webhook service gets `write` on one project.

Deleting a project (`DELETE /v1/projects/{id}`, admin scope) cascades to every ticket, comment, dep, and event under it, but does **not** touch tokens: a token scoped to a now-deleted project keeps existing and simply stops resolving against it. Revoke such tokens separately with `DELETE /v1/tokens/{id}` when you want them gone.

## Minting and management

Two equivalent paths, sharing the exact same store logic (hash at rest, plaintext shown once):

**CLI, local to the server (the original root of trust):**

```
takomo token create --actor agent:runner-1 --scopes read,write --projects rvp --expires 90d
takomo token list
takomo token revoke <token-id>
```

**HTTP, admin-scoped (added for one-command onboarding):**

| method & path            | scope | purpose |
|--------------------------|-------|---------|
| `POST /v1/tokens`        | admin | mint a token; body `{actor, scopes:[...], projects:[...]｜"*", expires_seconds?, rate_limit?}`. Returns the plaintext ONCE plus metadata; only the SHA-256 is stored. |
| `GET /v1/tokens`         | admin | list token metadata (id, actor, scopes, projects, created_at, expires_at, revoked_at, last_used_at). **Never** the plaintext or hash. |
| `DELETE /v1/tokens/{id}` | admin | revoke by token id. |
| `GET /v1/whoami`         | any valid token | echo the caller's own actor, scopes, and projects. |

The `takomo token create｜ls｜revoke`, `takomo whoami`, and `takomo init` CLI verbs wrap these.

### Deliberate posture shift (bounded relaxation)

The original v1 posture was: **token minting requires shell access to the server — that is the root of trust; the HTTP API's admin scope covers only projects/workflows.**

That is relaxed here on purpose: **admin scope can now mint, list, and revoke tokens over HTTP.** This is a conscious, bounded call, not an accident:

- An `admin` token could already create projects and upload arbitrary workflows over HTTP — capabilities at least as powerful as minting a scoped worker token. Letting admin also mint tokens does not hand an admin holder materially more reach than it already had.
- It is the enabler for one-command onboarding (`takomo init`): a repo goes from nothing to a provisioned project + a scoped `read,write` agent token without anyone SSHing to the server. That removes the single biggest onboarding friction.
- The blast radius is still gated by the `admin` scope. Ordinary `read,write`/`human`/`autoland` tokens get `403 auth.scope` on all three token-admin endpoints (only `whoami` is open to any valid token). Guard the admin token accordingly, and prefer short `expires_seconds` for admin tokens handed to automation.
- What did **not** change: plaintext is still shown exactly once and stored only as a SHA-256 hash; revocation and expiry are unchanged; the CLI-against-the-DB path still exists as the ultimate root of trust for bootstrapping the very first admin token.

## Share tokens (read-only web links)

A **share** mints a second, distinct kind of bearer token (`tks_`-prefixed, hashed at rest exactly like a normal token, plaintext shown once) that grants a **scoped, read-only, auto-expiring** view of the HTML board. It exists so a person can hand someone a link to a board without minting them a real account/token.

| method & path                    | auth | purpose |
|----------------------------------|------|---------|
| `POST /v1/shares`                | normal token, `write` scope | mint a share; body `{kind:"project"｜"epic", ref, ttl_seconds?}`. Returns the `token` ONCE plus a `path` (`/board#s=<token>`). |
| `GET /v1/shares`                 | normal token, `read` scope | list share metadata (admin sees all; else only the caller's own). Never the token or hash. |
| `DELETE /v1/shares/{id}`         | normal token, `write` scope | revoke (creator or admin). |
| `GET /v1/shares/self`            | **share token** | the share's scope + the project workflow (to render columns). |
| `GET /v1/shares/self/tickets`    | **share token** | the tickets in scope, read-only (`?include_archived=true` to include archived). |
| `GET /v1/shares/self/tickets/{id}` | **share token** | one in-scope ticket + comments/deps, for the detail panel. |

**Two scopes.** `kind:project` covers every ticket in a project. `kind:epic` covers a root ticket plus its FULL recursive descendant subtree (walked via `parent`, the same recursive-CTE the roadmap uses — any ticket can be the root; `epic` is just the common case). The stored/echoed kind for the subtree case is `subtree`.

**Distinct auth path.** A share token is validated only against the `shares` table and reaches ONLY the `/v1/shares/self*` endpoints. It is **read-only** and **cannot**: read arbitrary projects, hit any normal endpoint, or write anything — a share token on `GET /v1/tickets` (or any write) is rejected `401`. Conversely a normal `tk_` token is not accepted on the `self*` endpoints.

**Expiry / revocation.** Every share has a hard `expires_at` (default 24h, cap 30d). An expired or revoked share token returns **`410 Gone`** on every `self*` endpoint, which the board turns into a friendly "this shared link has expired" page.

**Fragment token, deliberately.** The mint returns `path = /board#s=<token>` — the token rides in the URL **fragment**, which browsers never send to the server, so it stays out of access logs and `Referer` headers. The board reads it from `location.hash`, never puts it in a query string, and never persists it.

**Tradeoff (accepted).** A share link is a bearer capability: **anyone with the link can view the scoped board, read-only, until it expires.** There is no per-viewer identity or audit. That is the point (frictionless read-only sharing), and it is bounded by: read-only, a single project/subtree scope, a mandatory expiry (≤30d), and one-command revocation. Prefer short TTLs and revoke when done; never mint a share over a project whose mere ticket titles/bodies are sensitive.

## Transport

The server binds localhost/tailnet and terminates plain HTTP; TLS is the deployment's job (Tailscale, reverse proxy, or platform TLS). The server refuses to bind non-loopback interfaces unless `TAKOMO_ALLOW_PUBLIC_BIND=1`, as a footgun guard.

## Rate limiting

Per-token sliding-window write budget (default 120 writes/min). Exceeding returns 429 with `Retry-After`. Purpose is not capacity (SQLite laughs at this load) but containing runaway agent loops; a 429 storm from one token is an anomaly worth surfacing in events (`kind: rate_limited` reserved for v1.1).
