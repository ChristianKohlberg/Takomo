# Development

Takomo is a Rust + axum server (single binary), a `bash` CLI, and a TypeScript MCP client, with docs and a Claude Code plugin in the same repo.

## Build, test, lint

Choose proportionate local checks using [Validation by impact and exposure](validation.md).
CI runs a fast lane on ordinary PRs/main pushes and full release/packaging checks
nightly, manually before release, and for packaging changes ([workflow](../.github/workflows/ci.yml)).

```sh
./scripts/build.sh                                 # Node 22 frontend, then Rust binary
cargo test --locked                                # fast integration suite (spawns real servers on ephemeral ports)
cargo clippy --all-targets -- -D warnings          # lint, warnings-as-errors
cargo fmt                                           # format (rustfmt.toml); CI runs --check
shellcheck -x clients/cli/takomo clients/cli/install.sh scripts/*.sh    # shell lint
(cd clients/mcp && npm ci && npm run build)         # MCP typecheck
```

Generated `web/dist/` assets are ignored by Git. Run the build script once on a
fresh checkout before standalone Cargo test or lint commands. After frontend
changes, rebuild the assets before compiling Rust.

The integration tests start real server instances against temporary SQLite DBs, so they cover the HTTP surface (workflow enforcement, claim/lease/fencing, the event log, the hosted `/mcp` endpoint) end to end. Tests are colocated in [`tests/`](../tests/).

## Layout

| Path | What |
|---|---|
| `src/` | The server: HTTP handlers (`src/api/`), the store + SQL (`src/store/`), auth, the hosted MCP endpoint (`src/mcp.rs`), the workflow engine, the board (`src/board.html`). |
| `clients/cli/takomo` | The `takomo` CLI (bash + curl + python3). |
| `clients/mcp/` | The TypeScript stdio MCP client (an alternative to the hosted `/mcp` endpoint). |
| `clients/claude-skill/` | The runtime + onboarding skills. |
| `plugins/takomo/` | The Claude Code plugin (skill + remote MCP), served from this repo as a marketplace. |
| `spec/` | The OpenAPI contract, the workflow format, and the auth model. |
| `workflows/` | Shipped workflow definitions, and the canonical source for them: `factory-default.yaml` (embedded by `src/workflow.rs`, what the server gives a project created without one) and `simple.yaml` (what the CLI's `takomo init` applies). Copies elsewhere are pinned to these by unit tests. |
| `scripts/` | Dev-loop helpers, e.g. the backlot `auth.token` hook. |
| `docs/design/` | **Historical** design records from July 2026 (adopt-vs-build evaluation, third-party surveys, the original build plan) — the reasoning behind the design, not a description of the current code. Every file carries a banner saying so; do not verify behaviour against them. |

## A running instance for manual testing — backlot

[backlot](https://github.com/ChristianKohlberg/backlot) (a build that accepts `backlot.yml`'s per-upkeep `timeout` and service `env_from` keys — see the end of this section) brokers a warm, running takomo for inspection or manual testing, so you don't hand-roll build/seed/serve. With `backlot` installed, from the repo root:

```sh
backlot up                    # build, seed a demo store, serve, print the URL + port
backlot token --role human    # a bearer token to paste into /board or /inbox
backlot ctx                   # the URL/ports an agent needs, as one blob
backlot run api               # the integration suite, with a classified verdict
backlot release               # return the environment to the pool, warm
```

The manifest is [`backlot.yml`](../backlot.yml). Two things it wires up are worth knowing:

**Seeded, not empty.** The `main` datastore has two presets. `dev` (the default for a
session lease) runs `takomo seed --preset dev`, which creates a `demo` project with ten
tickets spread across every workflow state — including claims, a dependency, and an epic —
plus questions of all four kinds, one advisory and one bounced back to its asking agent.
That is what makes `/board` and `/inbox` worth looking at. `empty` (the default for a
`backlot run` lease) just migrates. `takomo seed` is idempotent on the project.

**A token you can actually use.** Every endpoint but `/healthz` needs a bearer token, so
`auth.token` points at [`scripts/backlot-token.sh`](../scripts/backlot-token.sh), which maps
a role onto takomo's scopes: `agent` → `read,write`; `human` → `+human`; `admin` → `+admin`;
and `expert` → `+expert:domain:billing,expert:domain:product`, which is the scope the
seeded `approve` question gates on (a plain `human` token is refused there, by design).
The hook prints the bare plaintext on stdout, because backlot takes stdout verbatim as the
token.

## Conventions

- Every new/changed HTTP route ships with an integration test and an `spec/openapi.yaml` update.
- The spec must stay a *valid* OpenAPI 3.1 document, not merely parseable YAML. CI runs `redocly lint` against [`spec/redocly.yaml`](../spec/redocly.yaml). Two traps, both invisible to a human reader: a comma inside an unquoted description in a flow mapping silently truncates the sentence and turns its tail into a junk key, and `nullable: true` is 3.0 syntax that 3.1 tooling ignores (write `type: [string, "null"]`).
- Errors are part of the contract: reject with a stable `code`, a `message` written for an LLM reader, and (for transitions) `allowed_transitions` + a `remedy`. Never fail silently.
- Keep the CLI shellcheck-clean and the MCP typecheck green.

The Vite `/v1` proxy forwards WebSocket upgrades as well as HTTP requests, so
specification collaboration and project updates work against `TAKOMO_DEV_API`
during local development. A connected preview should show the normal save state
and collaborators rather than appear offline merely because it uses Vite.

## Fast frontend iteration and leased browser verification

Run `npm run dev:backlot` in `web/`. The helper finds the repository root,
leases only `server` for **15 minutes**, then starts local Vite with its API and
WebSocket proxy aimed at that lease. Open Vite's URL. Frontend saves are watched
locally and do not require `backlot sync` or a Rust rebuild. The local Vite process
ends with Ctrl-C; explicitly `backlot release` from the repository when finished.
The helper preserves an existing repository lease; it does not release it on exit.
Refresh a long session with `backlot up server --ttl 15`.

Ordinary `backlot up` and `sync` build frontend assets before Rust embeds them.
The build helper fingerprints frontend inputs and generated output, caches npm
installation in `node_modules`, and checks Cargo's own incremental cache. The declared
`target`, `node_modules` and generated `dist` directories survive `reset-data` cleanup. `--pristine` and
recycling discard the environment tree, including those caches. A first build
may take up to the manifest's 1800-second upkeep budget; warm frontend iteration
uses Vite instead.

`backlot run web` runs TypeScript, lint and component tests. For a real browser
against the leased application, install Chromium once with
`cd web && npx playwright install chromium`, then run `backlot run browser` from
the repository root. This check seeds its own run lease, mints a human token,
checks authenticated API data, and waits for a seeded ticket to render on the
board. It does not touch a separately held session. `backlot run api` still runs
the complete Rust suite, whose tests spawn their own servers.

The optional document-agent credential is allowlisted by `services.server.env_from`.
Export `TAKOMO_TENSORX_API_KEY` before `backlot up`; the caller supplies it only to
that lease. No daemon restart is needed. These manifest fields require a Backlot
build supporting per-upkeep `timeout` and service `env_from`.
