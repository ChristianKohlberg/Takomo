# Development

Takomo is a Rust + axum server (single binary), a `bash` CLI, and a TypeScript MCP client, with docs and a Claude Code plugin in the same repo.

## Build, test, lint

CI runs the same gates on every push and PR ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) — keep them green:

```sh
cargo build --release                              # build the binary (takomo)
cargo test --release                               # integration suite (spawns real servers on ephemeral ports)
cargo clippy --all-targets -- -D warnings          # lint, warnings-as-errors
cargo fmt                                           # format (rustfmt.toml); CI runs --check
shellcheck clients/cli/takomo clients/cli/install.sh   # shell CLI lint
(cd clients/mcp && npm ci && npm run build)         # MCP typecheck
```

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
| `workflows/` | Shipped workflow definitions (`simple.yaml`). |
| `scripts/` | Dev-loop helpers, e.g. the backlot `auth.token` hook. |
| `docs/design/` | The adopt-vs-build evaluation and architecture notes. |

## A running instance for manual testing — backlot

[backlot](https://github.com/ChristianKohlberg/backlot) (≥ 0.7) brokers a warm, running takomo for inspection or manual testing, so you don't hand-roll build/seed/serve. With `backlot` installed, from the repo root:

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

## In-session quality gates — handrail

[handrail](https://github.com/ChristianKohlberg/handrail) gates in [`.handrail/`](../.handrail/) surface project norms *in-session* — they guide, they don't enforce (CI is the wall): new/changed HTTP routes should ship with an integration test, `spec/openapi.yaml` should track route changes, and `cargo fmt`/`clippy` stay clean. With `handrail` installed: `handrail list`, `handrail run --changed`.

## Conventions

- Every new/changed HTTP route ships with an integration test and an `spec/openapi.yaml` update.
- Errors are part of the contract: reject with a stable `code`, a `message` written for an LLM reader, and (for transitions) `allowed_transitions` + a `remedy`. Never fail silently.
- Keep the CLI shellcheck-clean and the MCP typecheck green.
