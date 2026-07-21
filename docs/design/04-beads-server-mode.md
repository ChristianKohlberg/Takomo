# Research: beads (bd) server mode as a central fleet service

Verified 2026-07-18 against gastownhall/beads (moved from steveyegge/beads; 25.4k stars). Current release v1.1.0 (2026-07-04); docs re-aligned to v1.1.0 on 2026-07-17. Anything older than ~March 2026 about beads server mode is likely stale (heavy churn 0.56→0.60→1.x).

## 1. Server mode mechanics

Two storage modes (docs/architecture/dolt.md):
- **Embedded (default, `bd init`)** — Dolt in-process inside `bd`, data in `.beads/embeddeddolt/`, single writer (file-locked), zero ops.
- **Server (`bd init --server` / `BEADS_DOLT_SERVER_MODE=1`)** — bd becomes a MySQL-protocol *client* of an external `dolt sql-server`; multiple concurrent writers.

bd normally **manages the server lifecycle itself** (auto-start per project, per-path port, PID/logs in `.beads/`). For a central server use `bd init --external` (or `dolt.auto-start: false`) so bd never tries to spawn/kill servers.

**Config** (`bd dolt set <key> <value>` → `.beads/metadata.json`; `--update-config` mirrors to `config.yaml`): `database`, `host` (default 127.0.0.1), `port` (default 3307), `user` (default root), `data-dir`. Password never in config — `BEADS_DOLT_PASSWORD` env or INI credentials file `~/.config/beads/credentials` keyed by `[host:port]`. Env: `BEADS_DOLT_SERVER_MODE/HOST/PORT/USER/SOCKET`, `BEADS_DOLT_PASSWORD`, `BEADS_DOLT_SERVER_TLS`, `BEADS_DOLT_SHARED_SERVER`.

**Remote hosts: explicitly supported.** Docs' own examples: `bd dolt set host 192.168.1.100 --update-config`; credentials examples `[beads.company.com:3307]`, `[10.0.1.50:3308]`; IAP-tunnel note. `bd dolt status` documents "externally-managed servers — either a remote dolt_server_host or a local server managed outside bd".

## 2. Supported topology vs. steering

- The docs' **primary cross-machine story is NOT a central server** — it's Dolt remotes (`bd dolt push/pull` against `refs/dolt/data` on the git origin, or DoltHub/S3/GCS) plus peer-to-peer federation. FAQ: server mode is for "multiple concurrent processes on one machine"; "distributed setups" → federation.
- Central-server-as-shared-service **works and is used in the wild but is second-class**. Documented central-server material covers one machine (shared-server mode `~/.beads/shared-server/`, port 3308; macOS LaunchAgent bound to 127.0.0.1). No doc page for a network-exposed multi-machine server.
- **Known issues**:
  - #2641 (closed): bd 0.60 auto-killed a systemd-managed shared dolt server (45+ DBs) on every `bd init`; `dolt.auto-start: false` was ineffective; undocumented `.beads/dolt/.beads-credential-key` required. Fixed via #2700/#2676.
  - #3895 (**open**): `BEADS_DOLT_SERVER_TLS` ignored by `bd init` and several runtime paths — breaks TLS-required servers (e.g. Hosted Dolt).
  - #4102 (open, updated 2026-07-11): **5–10s per bd command** against a remote server over VPN — cold connection per CLI process (TLS handshake + auth each time) plus 10–15 serial SQL roundtrips per command.
  - #3239 (open): WAN latency amplification over Tailscale (100–200ms RTT) from per-ID checks during hydration.
  - #2922 (closed): `bd init --database` overwrote project_id on a shared remote server.
- Experimental in-flight: `bd init --proxied-server` (+ `--proxied-server-external-host/-port/-tls`) — a per-workspace local proxy fronting an external dolt server; looks like the emerging blessed path for remote servers (would amortize connections, addressing #4102).

**Net:** remote host:port works over plain MySQL protocol; fine on LAN, painful over WAN today; TLS env handling buggy as of #3895.

## 3. TLS / encryption

- `dolt sql-server` supports TLS natively (`tls_key`, `tls_cert`, `require_secure_transport`, `ca_cert`, `require_client_cert`).
- Beads' `BEADS_DOLT_SERVER_TLS` is not honored everywhere (#3895); beads docs say nothing about securing a network-exposed server. Community practice: SSH/IAP tunnels or VPN/Tailscale.

## 4. Multi-project on one server

Yes. One dolt sql-server hosts many databases; each beads project = one database (`database` key, default = issue prefix). Each project **must have a unique prefix**; a project-identity check refuses duplicate connects. `bd init --database <name>` attaches to an existing server DB. Real-world scale: 45+ DBs (#2641), 27 repos (#3895). `bd dolt clean-databases` exists because stale DBs waste server memory.

## 5. Auth granularity

- bd itself knows one `user` + password per host:port — no per-agent credential concept.
- But dolt implements MySQL grants: `CREATE USER`/`GRANT` at global, database, and table level, persisted in `.doltcfg/privileges.db`. Different agents can get different MySQL users incl. read-only; each agent's bd config sets its own `user` + credentials-file password. Only `mysql_native_password` auth. First start auto-creates passwordless `root@localhost` (localhost-scoped).

## 6. Operational footprint

- `dolt` = single Go binary; official Docker image `dolthub/dolt-sql-server`. Beads requires Dolt ≥ 2.2.0. bd = single static binary. No `bd daemon` in v1.1.0 (removed).
- No published memory sizing. dolt `max_connections` default 1000; bd pool `dolt.max-conns` default 10.
- dolt does not auto-discover config.yaml — pass `--config` explicitly. Run the central server under systemd yourself; set `bd init --external` on every client so bd's auto-start/auto-kill lifecycle never touches it.

## 7. Real-world central-server users

No official blog posts, but concrete users in the tracker: systemd shared server with 46 repos (#2641); 27 repos against DoltHub's paid Hosted Dolt over TLS (#3895); remote server over VPN (#4102); Tailscale-networked server with Mac laptop clients (#3239); self-hosted VPS dolt (#2515, declined Hosted Dolt at ~$600/yr). Pattern: it works, they persisted, each hit lifecycle/TLS/latency edges and filed issues.

## Bottom line

A fleet-central beads server is **technically supported and workable today in the shape: LAN or tunneled (Tailscale/SSH) `dolt sql-server` under systemd + `bd init --external --database <name>` per project + MySQL users per agent.** It is not the paved road: docs steer multi-machine users to git-remote sync/federation, TLS handling has an open bug, per-command connection overhead makes raw-WAN use slow, and beads' 4-status/no-state-machine model is unchanged by server mode. Dolt is the only storage backend in v1.1.0 (SQLite/Postgres backends exist only as post-tag work on main).
