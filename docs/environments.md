# Environments — where the software runs

A test result is a claim about a running system. An agent handed "verify this
behavior" needs the URL, the way to bring the thing up, and whether it is safe to
write to it — and without a registry all of that travels out of band and goes
stale silently.

An environment is that context, in the store, where an agent reads it before it
runs anything.

```sh
# what a runner reads before it runs anything
GET /v1/projects/{project}/environments
```

```json
{ "slug": "staging",
  "kind": "staging",
  "base_url": "https://staging.example.com",
  "bring_up": "backlot up --ttl 15",
  "teardown": "backlot release",
  "data_state": "production_like",
  "writable": true,
  "credentials_hint": "env:STAGING_TOKEN",
  "notes": "Reseeded 03:00 UTC — a result reported just before that is worth re-running." }
```

## Takomo stores; the agent computes

The same rule the rest of Checklist runs on, and the reason this file is short.
Nothing here is executed, polled, deployed or health-checked.

`bring_up` and `teardown` are **prose**, deliberately, not a structured command
spec. The server never runs them, so structure would be a promise it cannot
keep — and the real answer differs per caller anyway
(`BACKLOT_HOLDER_PID=$$ backlot up` for a person at a terminal,
`backlot up --ttl 15` for an agent). Prose is the only honest container.

`teardown` earns its own field because "who gives the lease back" is the thing
runners actually get wrong.

## `credentials_hint` is a pointer, never a credential

Any token with `read` can list environments. A secret here would be a secret
handed to every reader, so the field holds **where** a credential lives:
`env:STAGING_TOKEN`, `op://vault/staging/agent`, a runbook URL.

The name is chosen to refuse on sight. The validator caps the length and rejects
anything containing `-----BEGIN`. That is **not** secret detection and does not
pretend to be — it catches the single most common way a private key gets pasted
into a text field. A heuristic that caught most pasted secrets would be worse
than none, because it would teach people it works.

## `writable` is advisory

It is what an agent reads before running a destructive case, not a guarantee:
Takomo executes nothing and can enforce nothing. `kind: production` defaults it
to false, which errs toward refusing a destructive run rather than permitting
one — a default and not a rule, so a project that really does test writes
against production can say `writable: true`.

## The slug is immutable

Checks and tool calls address an environment by slug, so renaming one would
silently break every reference to it. `PATCH` does not accept the field at all.
A new name is a new environment, and the old one is archived.

`kind` is an enum for a related reason: a project that grows `prod`,
`production` and `Prod` can no longer answer "is it verified in production".

## Archived, never deleted

History may still name a decommissioned box, so `DELETE` archives. An archived environment leaves the default list,
stays readable by id, and comes back with `POST /v1/environments/{id}/unarchive`
— archiving changed nothing else about it, so restoring is a pure reversal.

## Who may write

`write`, like tickets and behaviors — **not** `human`. An agent that has just
leased an ephemeral preview instance is exactly the caller this registry exists
for, and gating registration on a person would push it straight back out of
band.

## Over MCP

```
takomo_environments        where can I run, and what may I do there   (read)
takomo_environment_file    register what I just stood up              (write)
```

`takomo_environment_file` upserts by `(project, slug)`, so a runner can call it
every run without accumulating duplicates. A field the caller omits keeps
whatever is already recorded — re-registering a URL must not silently erase
someone else's notes.

## From a shell

```sh
takomo env ls
takomo env new staging --kind staging --base-url https://staging.example.com \
  --bring-up 'backlot up --ttl 15' --teardown 'backlot release' \
  --credentials-hint env:STAGING_TOKEN
takomo env show ID   ·   takomo env set ID --read-only   ·   takomo env rm ID
```

The CLI refuses a pasted key in `--credentials-hint` **locally**, before the
request is sent. The server refuses it too, but by then the key has crossed the
wire and possibly landed in a log — so the check is worth having twice.

## An environment is not a configuration profile

Related, and not the same. An environment is **where** the application runs.
A configuration profile is **how it is configured**. Neither is modelled on
verification results today (`docs/verification.md`), and the two should not be
merged when they eventually meet.

See also: [`verification.md`](verification.md), [`spec/openapi.yaml`](../spec/openapi.yaml).
