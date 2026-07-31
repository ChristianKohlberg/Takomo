# Connecting hosted MCP clients (claude.ai, ChatGPT, Gemini)

takomo's MCP endpoint is `https://<your-host>/mcp`. A **local** client (Claude Code, Codex, Gemini
CLI) can be handed a bearer token directly and is done in one command. A **hosted** one — claude.ai,
ChatGPT developer mode, the Gemini app — can only be given a *URL*, and expects to negotiate its own
credential over OAuth. That is what the built-in authorization server is for
([spec/auth.md](../spec/auth.md#oauth-21-for-hosted-mcp-clients) covers its design; this page is the
wiring).

## One prerequisite: `TAKOMO_PUBLIC_URL`

```sh
TAKOMO_PUBLIC_URL=https://takomo.example.com   # public origin: no path, no trailing slash
```

Unset, the OAuth endpoints answer `404` and hosted clients cannot connect. Set it in your platform's
environment (Render: Dashboard → Environment; Docker: `-e`), restart, and check the startup line:

```
takomo v0.3.0 listening on http://0.0.0.0:8080 (db: /var/data/takomo.db)
  OAuth issuer https://takomo.example.com (resource https://takomo.example.com/mcp)
```

Verify from outside, without any credential — these two documents are what a client reads first:

```sh
curl -s https://takomo.example.com/.well-known/oauth-protected-resource | jq
curl -s https://takomo.example.com/.well-known/oauth-authorization-server | jq .issuer
# and the challenge that starts discovery:
curl -si -X POST https://takomo.example.com/mcp -d '{}' | grep -i www-authenticate
```

The value must be the origin **exactly** as users type it into their client. A client compares the
issuer against the URL it fetched discovery from, and the `resource` against the URL entered, byte
for byte; a trailing slash or a `/takomo` path prefix is a connection that fails with
"couldn't reach the MCP server" and no further explanation. takomo refuses the malformed forms at
startup rather than let you find that out from someone else's error log.

## What the human does, once per client

Every hosted product runs the same flow, and from the person's side it is:

1. Add the connector with the URL `https://takomo.example.com/mcp`.
2. Click connect. The product registers itself and opens takomo's consent page.
3. **Paste a takomo token** and approve. This is the credential the connection will act as.

The client never receives the token that was pasted. takomo issues it a *separate* one: same actor, a
subset of the scopes (whatever is left checked), the same project allowlist, the same write budget,
plus a one-hour expiry and its own entry in the token listing, tagged with the client it belongs to.
So you can end one connector without touching anything else.

There are two ways to list and revoke, and which one you have depends on how the instance is hosted.
Over HTTP, with an admin token — the path to use on a PaaS like Render, where there is no convenient
shell:

```sh
export TAKOMO_URL=https://takomo.example.com TAKOMO_TOKEN=tk_admin...
takomo token ls                 # a CONNECTION column appears; find the client's row
takomo token revoke tok_xxxxxxxx
```

Or on the box itself, against the database file:

```sh
takomo --db /var/data/takomo.db token list    # same CONNECTION column
takomo --db /var/data/takomo.db token revoke tok_xxxxxxxx
```

**Identify the row by its CONNECTION, not by its expiry.** The column names the client the row
belongs to (`Claude`, `ChatGPT`, …, or the `client_id` for a client that registered nameless), and
`GET /v1/tokens` carries the same thing as `oauth_client`. An expiry does not identify anything — the
consent token below is minted with `--expires 90d` — and two connectors approved by the same person
are otherwise identical rows: same actor, same scopes, same projects.

The column is present only when some row *is* a connection, so on an instance with no connectors the
listing looks exactly as it always did.

Getting it wrong is not recoverable, so it is worth the second look. Revoking the derived row ends
that one connection; revoking the token you approved with ends **every** connection approved with it,
plus that person's own access. Neither can be undone — the human re-approves.

Either way it is a real cut, not a one-hour inconvenience: the connection's refresh token is revoked
along with the credential, so the client cannot answer the 401 by rotating into a new one. Other
connections, including others approved by the same person, keep working.

Mint the token you will paste for exactly this purpose, rather than reusing an admin one:

```sh
takomo --db /var/data/takomo.db token create \
  --actor human:you --scopes read,write,human --projects thc-sourcing --expires 90d
```

Same flags on `takomo token create` if you are working over HTTP rather than on the box.

**Revoking that token also ends every connector consented with it** — one `takomo token revoke` cuts
the human's own access and every connection approved with it, in one move. Letting it **expire** does
not: an expiry is bookkeeping, and a connected client is meant to stay connected, so a stale
`--expires` flag does not become an outage months later. The flip side is worth knowing before you
rely on it: a short-lived consent token does not time-box the connection it approved. Ending one is a
revocation, and there are two to choose between — the connector's own row for that connection alone
(above; reach for this one when a single client misbehaves), or the consenting token for all of them.

`admin` is never granted through consent, whatever you paste — it is not offered on the page and not
honoured if a client asks for it. A connector that needs to create projects or mint tokens is a
connector that should be holding a token directly instead.

## Per product

### claude.ai (web, desktop, mobile)

Settings → Connectors → **Add custom connector** → URL `https://takomo.example.com/mcp`. Leave the
OAuth client ID and secret fields empty: takomo supports dynamic client registration, so Claude
registers itself. Approve on the consent page.

Claude's callback is `https://claude.ai/api/mcp/auth_callback`, registered automatically. Claude
refreshes reactively on a 401 and proactively up to five minutes before expiry, both of which the
token endpoint handles.

If it fails to attach, the usual cause is discovery rather than the flow: confirm the two
`.well-known` documents are reachable **unauthenticated** from the public internet, and that
`issuer` equals the origin you entered. Anthropic's traffic egresses from `160.79.104.0/21` if you
gate access at the edge — and note that a WAF in front of the host can break discovery even while
`/mcp` itself is reachable.

### ChatGPT (developer mode)

Settings → Connectors → Advanced → Developer mode → **Create**. URL
`https://takomo.example.com/mcp`, authentication **OAuth**. ChatGPT then discovers the authorization
server, registers, and sends you to the consent page.

Do **not** pick "No authentication" and hide a token in a proxy instead: ChatGPT's egress cannot be
meaningfully allowlisted, so that setup makes the URL the password on a store that accepts writes.

### Gemini

- **Gemini CLI** — no OAuth needed; it can send a header. `~/.gemini/settings.json`:

  ```json
  {
    "mcpServers": {
      "takomo": {
        "httpUrl": "https://takomo.example.com/mcp",
        "headers": { "Authorization": "Bearer tk_your_token" }
      }
    }
  }
  ```

- **Gemini app** (gemini.google.com) — Settings → Connected apps → add a custom app by MCP server
  URL. It takes a URL only, so the OAuth flow is what makes it work. Availability is limited
  (US personal accounts, "Keep Activity" on), so check it applies to you before debugging.

- **Gemini Enterprise** — supports a Streamable HTTP MCP server as a custom data store; same URL,
  same flow.

### Local clients, for contrast

No OAuth involved — hand them a token:

```sh
claude mcp add --transport http takomo https://takomo.example.com/mcp \
  --header "Authorization: Bearer tk_your_token"
```

Or the Node stdio wrapper in [`clients/mcp`](../clients/mcp/README.md), which is the right choice
when you want the fence-tracking convenience verbs.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `404 temporarily_unavailable` on a `.well-known` path | `TAKOMO_PUBLIC_URL` is unset. The body names it. |
| Client reports it cannot reach the server, no request in your logs | Discovery never resolved. Check both `.well-known` documents are reachable without a token, and that a WAF is not blocking the vendor's user agent. |
| The consent page says the token is not recognized | Truncated paste, or the token belongs to a different instance. Mint a fresh one. |
| The consent page says "Nothing would be granted" | The pasted token carries none of the checked scopes. Uncheck what it does not have, or use a different token. |
| `invalid_grant` immediately after approving | The code is single-use and lives ~60s. A client retrying an exchange lands here; start a fresh authorization. |
| Connector worked, then stopped, and `invalid_grant` says the grant was **already redeemed** | Refresh-token reuse was detected — the same refresh token was presented twice — and the whole family was revoked. Reconnect. If you did not trigger it, treat the credential as compromised; it is already dead. |
| Connector worked, then stopped, and `invalid_grant` says **this connection was ended at the server** | Not a theft signal. Someone revoked this connection's token (`takomo token revoke`, or `DELETE /v1/tokens/{id}`), or reuse on a sibling credential took the whole connection down with it. Reconnecting works, but find out who ended it and why first. |
| Connector stopped, and `invalid_grant` says the credential that consented was revoked | The token pasted at the consent screen has been revoked (or deleted), which ends every connector derived from it. Approve again with one that has not — and if you did not expect it, ask whoever revoked it first. An *expired* consenting token does not do this. |
| `415` from `/oauth/token` | Something is sending JSON. The token endpoint takes `application/x-www-form-urlencoded` (RFC 6749 §4.1.3). |
