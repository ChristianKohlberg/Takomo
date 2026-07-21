# Takomo

[![CI](https://github.com/ChristianKohlberg/Takomo/actions/workflows/ci.yml/badge.svg)](https://github.com/ChristianKohlberg/Takomo/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

**A task tracker you host, that every AI agent, orchestrator, and human on a project talks to over HTTP** — one source of truth for work, instead of a todo list trapped in a single checkout.

Repo-embedded trackers keep a nice trail inside one repository, but parallel work across many orchestrators, machines, and checkouts needs one authority everyone can reach. Takomo is that authority: hierarchical tickets, a real per-project state machine, atomic claim/lease so exactly one worker owns a ticket, and an append-only event log — with errors written to teach an LLM what to do next.

![Takomo board — the read-only /board view](docs/images/board-preview.png)

> **There is no public Takomo instance — you run your own.** It's a single Rust + SQLite binary; standing one up is a click or a container (below). Once it's running, the Claude Code plugin and the `takomo` CLI point at *your* host.

## 1. Host it

Takomo is a server you run yourself. Pick one:

**Option A — Deploy to Render (one click):**

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/ChristianKohlberg/Takomo)

The [`render.yaml`](render.yaml) Blueprint provisions a web service with a persistent disk (SQLite durability), TLS, and a `/healthz` check, and gives you a `https://<name>.onrender.com` URL.

**Option B — Docker (anywhere):**

```sh
docker build -t takomo .
docker run -d -p 8080:8080 -v takomo-data:/var/data --name takomo takomo
```

Put TLS in front of it — Takomo terminates plain HTTP and refuses non-loopback binds unless `TAKOMO_ALLOW_PUBLIC_BIND=1`.

**Then mint the first admin token** with shell access to the server (the root of trust) — `render ssh` on Render, `docker exec` locally:

```sh
takomo --db /var/data/takomo.db token create \
  --actor human:me --scopes read,write,human,admin --projects '*'
```

It prints a `tk_...` token **once**. Everything below points at `https://<your-host>/v1` with a token. Local builds, TLS, and off-box backups: [docs/hosting.md](docs/hosting.md).

## 2. Use it from Claude Code — and any MCP client

Takomo hosts an **MCP server inside the binary** (streamable HTTP at `/mcp`), so any MCP-capable client connects directly — no local process to run.

**Claude Code (two lines)** — this repo doubles as the plugin marketplace:

```
/plugin marketplace add ChristianKohlberg/Takomo
/plugin install takomo
```

That installs the `takomo` skill (teaches the agent to use the store as its source of truth) plus the remote MCP server. Set your host + token before launching Claude Code — nothing is baked into the plugin:

```sh
export TAKOMO_URL="https://<your-host>/v1"
export TAKOMO_TOKEN="tk_<your read/write token>"
```

**Any other MCP client (Codex, Cursor, …)** — point it at the endpoint with a bearer token, e.g. adding it to Claude Code by hand:

```sh
claude mcp add --transport http \
  --header "Authorization: Bearer tk_<token>" \
  takomo https://<your-host>/mcp
```

The agent then gets native `takomo_new / ready / next / start / done / comment / link / dep / roadmap / …` tools. See [plugins/takomo/README.md](plugins/takomo/README.md) and [clients/mcp/README.md](clients/mcp/README.md).

## 3. Use it from the CLI

`takomo` is a self-contained `bash` + `curl` + `python3` script — for humans, and for agents/harnesses without MCP. One line:

```sh
curl -fsSL https://raw.githubusercontent.com/ChristianKohlberg/Takomo/main/clients/cli/install.sh | sh
takomo help
```

The installer checks `curl`/`python3` and puts `takomo` (and a short `tk` alias) on your `PATH`. Reference: [clients/cli/README.md](clients/cli/README.md).

## 4. Quick start

**Onboard a repo — one command.** From the root of any git repo, with an admin token:

```sh
export TAKOMO_URL="https://<your-host>/v1"   # note the /v1
export TAKOMO_TOKEN="tk_<admin token>"       # used only to provision

takomo init                    # or: takomo init myproject --workflow simple
```

`takomo init` creates the project (named after the repo), applies the `simple` workflow, mints a `read,write` token scoped to just that project, and writes `.takomo/config` (`url` + `project`, safe to commit) and `.takomo/token` (mode `600`, auto-gitignored). After that `takomo` auto-loads `.takomo/` by walking up from your cwd — no env setup inside the repo.

**Work the queue:**

```sh
takomo whoami                          # who am I: actor, scopes, projects
takomo new "Wire up the frobnicator"   # create a ticket (warns about likely duplicates)
takomo ready                           # what's claimable right now
takomo roadmap                         # epic progress: a bar + child counts per epic
ID=$(takomo next | awk '{print $2}')   # atomically claim the next ready ticket
takomo start "$ID"                     # -> in_progress (the lease fence is remembered for you)
takomo comment "$ID" "opened PR, waiting on CI"
takomo link "$ID" --pr https://github.com/org/repo/pull/42
takomo done "$ID"                      # -> done (claim auto-releases)
```

Every rejection teaches: an illegal move returns the allowed transitions and a remedy, and `takomo` prints them. Never retry a rejected call unchanged.

## What you get

- **Enforced state machine.** Per-project workflows with server-enforced transitions; illegal moves return a teaching `409` (current state, allowed transitions, remedy) written for an LLM.
- **Atomic claim/lease.** A monotonic fencing token guarantees exactly one worker owns a ticket; expired leases return it to the ready queue automatically.
- **Hierarchy + dependencies.** `epic → task/bug/subtask` trees, `blocked_by` edges (reverse + transitive views), labels, roadmaps, and free-form namespaced JSON metadata.
- **Append-only event log.** A durable `?since=<seq>` cursor plus an SSE stream — the audit trail and the wake feed in one.
- **Read-only web board** at `/board`, with scoped, expiring share links.
- **Archiving + anti-lock-in.** Archive terminal tickets out of view; JSONL export/import with idempotent re-import, plus importers for beads and beans.
- **Single binary.** Rust + SQLite (WAL); one process, zero external services.

The default **`simple`** workflow is a drop-in for beads/beans: `draft → todo → in_progress → done`, with `blocked`/`cancelled` escape hatches, `todo` claimable, and no human-approval gates — so one person or a fleet of agents just works the queue. Richer per-project workflows (approval gates, autoland) are data you upload: [spec/workflow-format.md](spec/workflow-format.md).

## Auth & security

Bearer tokens (`tk_...`), scoped (`read`/`write`/`human`/`autoland`/`admin`), hashed at rest, shown in plaintext once. One token per actor — never shared. Tokens are the whole perimeter: keep them out of git (`takomo init` gitignores `.takomo/token`), rotate by revoke-and-remint, and scope narrowly so a leak is contained. Share links are read-only and expiring but **unauthenticated** — treat any link as public for its lifetime. Full model: [spec/auth.md](spec/auth.md).

## Docs

- [docs/hosting.md](docs/hosting.md) — self-hosting depth: local build, TLS, and off-box backups (Litestream).
- [docs/development.md](docs/development.md) — building, testing, linting, and the `backlot`/`handrail` dev loop.
- [spec/openapi.yaml](spec/openapi.yaml) — the full v1 HTTP API (tickets, workflows, claims/leases, tokens, event log, `/mcp`).
- [spec/workflow-format.md](spec/workflow-format.md) — the per-project state-machine format.
- [spec/auth.md](spec/auth.md) — tokens, scopes, and the auth model.
- [clients/cli/README.md](clients/cli/README.md) · [clients/mcp/README.md](clients/mcp/README.md) · [plugins/takomo/README.md](plugins/takomo/README.md) — the client surfaces.
- [docs/design/](docs/design/) — the adopt-vs-build evaluation, architecture, and DX-gap notes.

## License

Apache-2.0. See [LICENSE](LICENSE); attribution in [NOTICE](NOTICE).
