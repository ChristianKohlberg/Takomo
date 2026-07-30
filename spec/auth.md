# Token scheme

Basic authentication done properly-but-simply: bearer tokens, scoped, hashed at rest, minted by CLI. No users and no sessions — the token *is* the identity.

There is now an OAuth 2.1 authorization server in front of `/mcp` (see [OAuth for hosted MCP clients](#oauth-21-for-hosted-mcp-clients) below), which is off unless an operator sets `TAKOMO_PUBLIC_URL`. It does not add a credential type or an identity model: what it issues is an ordinary `tk_` token with an expiry, derived from one a human already holds. Everything in this document about scopes, project allowlists, budgets and revocation applies to it unchanged.

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
| `admin`   | projects (create **and delete**), workflow upload, token management, force-releasing a claim (`POST /v1/tickets/{id}/force-release` — takes a ticket from a holder that is gone, bumping the fence so the displaced worker's next write 409s) |
| `answer:relay` | record an answer a human made **out of band**, by sending `on_behalf_of` on `POST /v1/questions/{id}/answer`. Records the named human as `answered_by` and the caller as `relayed_by`. It is not `human`: it cannot answer as itself, cannot relay a question the same actor asked (`answer.relay_self`), and cannot touch an `approve` question at all (`answer.relay_approve`) |

Typical grants: workers get `read,write` on their project; orchestrators get `read,write` plus `autoland` where yolo applies, and `answer:relay` if they are expected to write down decisions a human gives them in conversation; humans get `read,write,human`; the webhook service gets `write` on one project.

### Why `answer:relay` is not just `human`

Answering a question is the human authorization gate, so it stays gated. But that gate was also blocking something it was never meant to: **transcription**. A human reads a parked question, decides, and tells the orchestrating agent — which then cannot write the decision down, so the question stays open and the ticket stays parked over bookkeeping.

`answer:relay` separates *deciding* from *recording it*. The audit trail keeps saying who decided (`answered_by`, and the `question_answered` event's actor), and gains `relayed_by` so a later reader can tell a relayed decision from a first-hand one. A relayed answer also performs the ticket's `scope:human` resume, on the named human's authority — the same way the timeout path enacts a recommendation with `human` implied. Recording a decision and then refusing to act on it would strand the ticket outside the ready queue while its question read as answered, which is worse than leaving it visibly blocked.

What makes it safe to hand to an agent is that it cannot manufacture a decision: relaying your own question is refused, and an `approve` question — whose whole point is that answering it *from a token holding `expert:<tag>`* is the evidence a named expert exercised their authority — is never relayable, not even by a relay token that happens to hold the expertise. A relayed name is a claim about who decided; the expert scope is proof.

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
| `GET /v1/shares/self/tickets`    | **share token** | one page of the tickets in scope, read-only (`?include_archived=true` to include archived; `?limit=` ≤200, `?cursor=`). |
| `GET /v1/shares/self/tickets/{id}` | **share token** | one in-scope ticket + comments/deps, for the detail panel. |

**Two scopes.** `kind:project` covers every ticket in a project. `kind:epic` covers a root ticket plus its FULL recursive descendant subtree (walked via `parent`, the same recursive-CTE the roadmap uses — any ticket can be the root; `epic` is just the common case). The stored/echoed kind for the subtree case is `subtree`.

**The scope is a type, and it fails closed.** `epic` is *request* vocabulary only: it is normalized to `subtree` before the row is written, and the reverse mapping accepts nothing but the two stored spellings. From the row read onwards the scope travels as a `ShareKind` enum — never a string compared against `"subtree"` — so choosing between the subtree query and the project-wide query is an exhaustive `match` with no "everything else" arm to fall into. A `shares.kind` value that is neither (only reachable by hand-editing the database) makes the share unreadable: every `self*` endpoint answers **`500 share.kind_unrecognized`** rather than serving it under the wider project scope. A share link is designed to be pasted around, so the failure direction is part of the boundary.

**Distinct auth path.** A share token is validated only against the `shares` table and reaches ONLY the `/v1/shares/self*` endpoints. It is **read-only** and **cannot**: read arbitrary projects, hit any normal endpoint, or write anything — a share token on `GET /v1/tickets` (or any write) is rejected `401`. Conversely a normal `tk_` token is not accepted on the `self*` endpoints.

**Expiry / revocation.** Every share has a hard `expires_at` (default 24h, cap 30d). An expired or revoked share token returns **`410 Gone`** on every `self*` endpoint, which the board turns into a friendly "this shared link has expired" page.

**Bounded work per request, and a budget per link.** Both halves matter, because either alone leaves the hole open: a cap on one response does not stop a script repeating it, and a rate limit does not stop one request from scanning a 100k-ticket project.

- `GET /v1/shares/self/tickets` returns a **page** — `limit` defaults to and is clamped at 200, `cursor` continues — and never truncates silently: `next_cursor` is non-null exactly when more remain, and such a page carries a `warning` naming the next call. Before this it returned the whole project with no `LIMIT` (takomo-vlpm).
- Every request on the `self*` routes is charged to a **per-share sliding window of 120 requests/minute**, shared by all viewers of that link. Over it is `429 share.rate_limited` with `Retry-After`; the link keeps working (takomo-fgca).

**Reads are charged here, deliberately unlike the `tk_` path**, where `GET` is free. The budgets answer different questions. A `tk_` token is a named actor, individually revocable, and what it can do wrong at scale is a runaway *write* loop. A `tks_` share is a bearer capability *built* to be pasted into a chat or an email, with no per-viewer identity, and reading is the only thing it can do — so charging only writes would leave it with no budget at all. Per link rather than per viewer for the same reason the tradeoff below is stated the way it is: the link is the identity, and revoking it is the lever an operator actually has.

An unrecognized share token is not charged (there is no share to charge it to); what bounds it is the work it causes, which is one indexed lookup by token hash before the `401`.

**Fragment token, deliberately.** The mint returns `path = /board#s=<token>` — the token rides in the URL **fragment**, which browsers never send to the server, so it stays out of access logs and `Referer` headers. The board reads it from `location.hash`, never puts it in a query string, and never persists it.

**Tradeoff (accepted).** A share link is a bearer capability: **anyone with the link can view the scoped board, read-only, until it expires.** There is no per-viewer identity or audit. That is the point (frictionless read-only sharing), and it is bounded by: read-only, a single project/subtree scope, a mandatory expiry (≤30d), and one-command revocation. Prefer short TTLs and revoke when done; never mint a share over a project whose mere ticket titles/bodies are sensitive.

## OAuth 2.1, for hosted MCP clients

**The problem it solves.** A *local* MCP client can send `Authorization: Bearer tk_...`. A *hosted* one — claude.ai, ChatGPT developer mode, the Gemini app — can only be handed a URL, and expects to negotiate a credential over OAuth. Without an authorization server the only ways to connect them are both bad: an authless proxy makes the URL the password on a store that accepts writes, and a token in a query string leaks through logs and proxies (and the MCP authorization spec prohibits it outright).

**Off by default.** The endpoints exist only when `TAKOMO_PUBLIC_URL` names the public origin the server is reached at (`https://takomo.example.com` — no path, no trailing slash; plain `http` is accepted only on loopback). Unset, they answer `404 temporarily_unavailable` naming the variable, and `/mcp` sends no `WWW-Authenticate` header. The value is validated at startup, not on first use, because a client compares the issuer and the resource identifier byte-for-byte against what it fetched and what the user typed — a stray path is not cosmetic, it is a connection that fails inside someone else's product.

| method & path | auth | purpose |
|---|---|---|
| `GET /.well-known/oauth-protected-resource` | **none** | RFC 9728 metadata: the resource is `<base>/mcp`, the authorization server is `<base>`. Also served at the `/mcp`-suffixed path a client probes. |
| `GET /.well-known/oauth-authorization-server` | **none** | RFC 8414 metadata. Advertises `S256` (required by the MCP spec so a client can check PKCE support up front) and `token_endpoint_auth_methods_supported: ["none"]`. |
| `POST /oauth/register` | **none** | RFC 7591 dynamic client registration. Public clients only — no `client_secret` is ever issued. |
| `GET /oauth/authorize` | **none** | the consent page (HTML, for a human in a browser). |
| `POST /oauth/authorize` | the token pasted into the form | submit consent; issues an authorization code by redirect. |
| `POST /oauth/token` | PKCE | `authorization_code` and `refresh_token` grants, `application/x-www-form-urlencoded`. |

Unauthenticated is not an oversight: these are what a client uses *in order to* obtain a credential. Before they existed these paths fell through to the `/v1` middleware and answered `401`, which is precisely the dead end a hosted client cannot get past.

**Consent without user accounts.** takomo has tokens, not users, and `client_credentials` is deliberately not offered — every credential here has to represent a specific human's decision. So the consent screen authenticates the human with **a takomo token they already hold**: they paste one, see which client is asking and for what, uncheck anything they do not want to hand over, and approve.

What the client receives is a **derived** token: same `actor`, the checked scopes intersected with the ones that token actually carries, the same project allowlist, the same write budget — plus a one-hour expiry and its own row in `takomo token list`. So it is:

- attributable — events and claims still name the human's actor, and `granted_by` records which token consented;
- revocable on its own, with `DELETE /v1/tokens/{id}`, without touching the human's own token;
- narrower than the credential that approved it, never wider.

An omitted `scope` parameter means "act as me": the consent screen offers everything grantable, pre-checked, and the intersection with the pasted token happens on approval. A scope the human *unchecks* stays unchecked — including across a re-render after a bad token, which is the one place a form could quietly hand back what was declined. `offline_access` is the exception, and it is not a checkbox at all: it grants no authority, a refresh token is issued either way, and the only thing unchecking it could mean — a connection that dies an hour after it is made — is a trap rather than a capability. The page states it as a sentence instead.

That is strictly better than the alternative on offer today, which is pasting a long-lived token into a client's own header field.

**Revoking the consenting token ends every connector derived from it.** The consent snapshot is frozen so it cannot widen, but it is not independent: every exchange and every refresh re-checks the token recorded in `granted_by` and answers `invalid_grant` (`ConsentWithdrawn`) if it has been revoked or deleted. Outstanding *access* tokens are not revoked by this — they expire within the hour, and `DELETE /v1/tokens/{id}` on the derived row is the way to cut one immediately.

**A consenting token that merely expires does not**, deliberately. The asymmetry is the design: a revocation is an operator deciding something must stop, while an expiry is bookkeeping typed once and forgotten, and letting it kill a working connector would turn a stale `--expires` flag into an outage without adding anything revocation does not already give. A connected client is meant to stay connected — the 30-day refresh window slides on every use, and nothing here introduces a second clock that does not.

The honest cost: **a short-lived consent token no longer bounds the connection it approved.** Approving with `--expires 7d` does not stop the connector in seven days. The two levers that do are revoking that token, which cascades, and revoking the connector's own row in `takomo token list`, which does not touch anything else.

**`admin` is never granted this way.** It is not offered on the consent screen and not honoured if requested, whatever the pasted token carries — and the common case *is* an admin token, because it may be the only one the operator has. `admin` mints tokens, creates and deletes projects, and force-releases other workers' leases: administration, not work. Consent narrows; it never widens. Grantable scopes are `read`, `write`, `human`, plus `offline_access` (which only asks for a refresh token and maps to nothing).

**PKCE is the client authentication.** Every client is public, `S256` is mandatory, and `plain` is refused. A wrong `code_verifier` is refused *without* consuming the code — otherwise observing a redirect would be enough to deny service to the real client.

**Authorization codes are single-use, ~60s.** A replay revokes everything that code already bought, because a replay cannot be distinguished from a stolen code racing the legitimate client. The spent row is therefore kept for an hour after redemption rather than swept at expiry: without it a replay matches nothing and reports "no such grant", which reads like a typo while the stolen credential keeps working.

**Refresh tokens rotate on every use** (OAuth 2.1 requires it for public clients) and carry a *family* id. Presenting an already-rotated one is reuse: the whole family descended from that consent is revoked, access tokens included. The superseded **access** token is deliberately left to expire on its own — clients refresh proactively, while requests may still be in flight on it, so revoking it would fail those to buy at most an hour.

**The open-redirect guard is the validation order.** `client_id` and `redirect_uri` are checked against the registration *first*; until both are known good, errors render as a page and nothing is redirected anywhere — not even an error. Redirect URIs are matched literally (a differing scheme, host, port, path or trailing slash is a different URI). Only afterwards are protocol errors reported the RFC 6749 way, by redirect.

**Registration is rate-limited globally.** It is unauthenticated by specification, so there is no credential to charge and no caller identity to key a window by: one global sliding window of 30/minute, `429 temporarily_unavailable` with `Retry-After` over it. A real deployment registers a handful of clients ever.

That budget *paces* an unauthenticated write; it does not bound the table, since 30/minute sustained is still tens of thousands of rows a day. What bounds it is the sweep: **a registration older than 24h with no `oauth_codes` and no `oauth_refresh` row referencing it is deleted.** So a client is protected for exactly as long as it still has a live credential — a connector in use keeps a rotating refresh token and never becomes sweepable, while one idle past the 30-day refresh lifetime loses that row to the same sweep and its registration goes on a later tick. That is the right outcome: the connection is already dead, and a hosted client registers again when it reconnects. Same for a flow started from a >24h-old registration that never completed one — the `client_id` is gone and the client re-registers, which is what these products do on a fresh connection anyway.

**Error bodies here are RFC 6749's, not takomo's.** `{"error", "error_description", "remedy"}` — an OAuth client parses `error` and nothing else, so takomo's `code` would be invisible to it. The vocabulary is listed under `x-oauth-errors` in `spec/openapi.yaml`, separately from `x-error-codes`, so the two namespaces are not mistaken for one.

For the per-product wiring (claude.ai, ChatGPT, Gemini), see [docs/hosted-mcp-clients.md](../docs/hosted-mcp-clients.md).

## Transport

The server binds localhost/tailnet and terminates plain HTTP; TLS is the deployment's job (Tailscale, reverse proxy, or platform TLS). The server refuses to bind non-loopback interfaces unless `TAKOMO_ALLOW_PUBLIC_BIND=1`, as a footgun guard.

OAuth raises the stakes on that: an issuer advertised over plain `http` would publish an authorization server whose tokens travel in clear text, so `TAKOMO_PUBLIC_URL` refuses a non-loopback `http` origin outright.

## Rate limiting

Per-token sliding-window write budget (default 120 writes/min). Exceeding returns 429 with `Retry-After`. Purpose is not capacity (SQLite laughs at this load) but containing runaway agent loops; a 429 storm from one token is an anomaly worth surfacing in events (`kind: rate_limited` reserved for v1.1).

Share tokens have their own window — 120 **requests**/min per link, `429 share.rate_limited` — because a read-only bearer capability designed to be pasted around has a different risk profile from a named actor's token. See "Share tokens" above for why reads are charged there and not here. Answer grants (`tka_`) need no window: single-use bounds them.

**Only writes are charged, on both surfaces.** On REST that is the HTTP method: `GET`/`HEAD` are free, everything else debits. On MCP the method says nothing — every frame is `POST /mcp` — so the budget is debited per tool call, by name: the read-only tools (`takomo_show`, `takomo_list`, `takomo_ready`, `takomo_deps`, `takomo_questions`, `takomo_projects`, `takomo_workflow`, `takomo_roadmap`, `takomo_whoami`) are free, and so are `initialize`, `tools/list` and an unknown tool name; every other tool debits one write. A tool added later is charged until it is explicitly declared a read. An MCP write that exhausts the budget comes back as a tool-level error carrying the same `rate.limited` code, message, remedy and `status: 429` — the transport reports MCP failures in the result body, not the HTTP status.
