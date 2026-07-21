# Changelog

All notable changes to takomo are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/), and the project aims
to follow [Semantic Versioning](https://semver.org/). The `/v1` HTTP API evolves
additively only.

## [0.1.0] — unreleased

First public release: a single-binary, self-hostable, hosted task tracker that
every AI agent, orchestrator, and human on a project talks to over HTTP.

### Server

- Single Rust/axum binary over SQLite (WAL) — HTTP server plus `token` and
  `project` admin subcommands.
- Hierarchical tickets (`epic` → `task`/`bug`/`subtask`) with single-parent
  trees, `blocked_by` dependency edges, labels, and free-form namespaced JSON
  metadata.
- Per-project, server-enforced state machine with a configurable workflow
  format; illegal transitions return a teaching `409` (current state, allowed
  transitions, and a remedy) written to be read by an LLM.
- Atomic claim/lease with a monotonic fencing token so exactly one worker owns a
  ticket; expired leases return the ticket to the ready queue.
- Append-only event log with a durable `?since=<seq>` cursor and an SSE stream.
- Ready queue (`GET /ready` peek, `POST /ready/claim` atomic take) driven by
  dependency readiness.
- Bearer-token auth, scoped (`read`, `write`, `human`, `autoland`, `admin`) and
  SHA-256 hashed at rest; token minting over both the server CLI and an
  admin-scoped HTTP surface.
- Read-only web board at `/board`, plus scoped, expiring share links.
- Archive support (additive, non-destructive startup migration).
- JSONL export/import with idempotent re-import; importers for takomo, beads,
  and beans.
- `/healthz` as the only unauthenticated endpoint; refuses non-loopback binds
  unless `TAKOMO_ALLOW_PUBLIC_BIND=1`.

### Clients

- `takomo` — a self-contained `bash` + `curl` + `python3` CLI over the REST API,
  with `takomo init` one-command repo onboarding and local fence tracking.
- Claude Code plugin (this repo doubles as the plugin marketplace): bundles the
  takomo skill and a remote MCP server declaration for the hosted endpoint.
- Model Context Protocol (MCP) server for agent harnesses.
- Agent skills for using the store as a source of truth and for onboarding a
  repo.

### Deployment

- Render Blueprint (`render.yaml`) with a persistent disk and health check.
- Portable `Dockerfile` for VM / self-host deployment.
- Prepared (opt-in) Litestream continuous backup to S3-compatible storage.
