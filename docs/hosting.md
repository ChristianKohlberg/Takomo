# Hosting Takomo

Takomo is a single Rust + SQLite binary you run yourself. The [README](../README.md) covers the two fast paths (Deploy-to-Render button, Docker); this is the depth: local builds, TLS, and off-box backups.

## The two supported deploys

**Render (Blueprint).** [`render.yaml`](../render.yaml) provisions a `rust` web service that builds the frontend with Node 22 and the Rust binary via `./scripts/build.sh`, serves on `$PORT`, mounts a 1 GB persistent disk at `/var/data` (SQLite durability across deploys), sets `TAKOMO_ALLOW_PUBLIC_BIND=1`, and health-checks `/healthz`. Render terminates TLS for you. Deploy with the button in the README or via Dashboard → New → Blueprint. The blueprint sets `autoDeployTrigger: "off"`, so a push to `main` does not redeploy the service: a release is a specific commit deployed by hand after a full CI run, per [Validation by impact and exposure](validation.md#release-and-deployment).

**Docker (portable).** The [`Dockerfile`](../Dockerfile) builds one self-contained image with Kroki, PlantUML/Salt, D2, Mermaid and [Litestream](https://litestream.io/) (dormant unless you set a bucket — see [Backups](#backups-litestream)).

### Docker runtime

```sh
docker build -t takomo .
docker run -d --name takomo --restart unless-stopped \
  --memory=3g --cpus=2 --pids-limit=512 \
  -p 8080:8080 -v takomo-data:/var/data takomo
docker exec --user takomo takomo takomo --db /var/data/takomo.db token create \
  --actor human:me --scopes read,write,human,admin --projects '*'
```

Diagrams work immediately without another container or service URL. This is a
larger image than the standalone Rust binary: Java, Node and Chromium are runtime
dependencies. The example budgets 3 GiB and two CPUs for the combined workload;
measure your own document sizes and concurrency when sizing production.

The supervisor starts as root to prepare persistent data and launch services under distinct UIDs;
Takomo and Litestream run as UID 10001, Kroki as 10002 and Mermaid as 10003, with
no-new-privileges. Do not override the container user: startup needs to select
these accounts. Use `docker exec --user takomo` for CLI administration to keep
new database files owned by the application. The supervisor does not run renderers
for `docker run ... takomo token ...` and other CLI subcommands.

Named volumes and provider-mounted disks, including Render disks owned by root
or an earlier runtime user, are prepared automatically. Startup makes `/var/data` private to
UID 10001 and repairs ownership of the selected database and its SQLite
`-wal`, `-shm` and `-journal` files. Existing bytes are preserved. The database's
own `.<name>-litestream` metadata tree is also preserved and made usable by
Litestream. Unrelated files and directories are not recursively changed.

This migration is limited to databases directly inside `/var/data`. `--db` takes
precedence over `TAKOMO_DB`; relative database paths use `/var/data` as their base.
For custom nested or external paths, prepare ownership and private permissions
for UID 10001 yourself. Symlinks, hard-linked files and nested mounts in the
selected database/backup state are rejected before ownership changes; startup
never follows them into other data. Keep backup credential mounts private to
UID 10001. Renderer accounts share the container kernel/network, so UID separation
is not equivalent to separate containers. Keep unrelated secrets out of
world-readable mounted files.

`docker stop` requests shutdown of all services. A main service failure terminates
the whole container with a nonzero status so `--restart unless-stopped` can
recover the bundle. The health check checks Takomo and both bundled services;
Docker health status alone does not trigger a restart for a still-running process.
For a custom `serve --bind`, the health probe follows that address and port.

A nonempty `TAKOMO_KROKI_URL` switches to an external renderer and avoids launching
bundled Java/Node/Chromium. See [diagram configuration](diagrams.md). The optional
Litestream restore/replicate behavior is retained in either mode.


## Building and running from source

One binary is both the HTTP server and the `token` / `project` admin CLI.

```sh
./scripts/build.sh
alias takomo=./target/release/takomo

# mint the first admin token (root of trust: shell access to the DB)
takomo --db takomo.db token create --actor human:me --scopes read,write,human,admin --projects '*'

# serve
takomo --db takomo.db serve --bind 127.0.0.1:8080
```

Requires a recent stable Rust toolchain. The DB file is created on first run.

## Binding and TLS

Takomo terminates **plain HTTP** and expects to sit behind TLS — a platform (Render), a reverse proxy (Caddy/nginx), or a private network (Tailscale). As a footgun guard it refuses to bind a non-loopback interface unless you opt in:

```sh
TAKOMO_ALLOW_PUBLIC_BIND=1 takomo --db takomo.db serve --bind 0.0.0.0:8080
```

`/healthz` needs no token — use it as your platform's readiness/liveness probe. Every API route requires `Authorization: Bearer tk_...`. The exception is the OAuth authorization server (`/oauth/*` and the two `.well-known` documents), which is unauthenticated by design when `TAKOMO_PUBLIC_URL` is set, because it is what a hosted client reads in order to obtain a token — see [spec/auth.md](../spec/auth.md#oauth-21-for-hosted-mcp-clients).

### WAF note

If your host sits behind a WAF that blocks the default `python-urllib` User-Agent (some edges do), library clients can get a `403` HTML block page instead of JSON. The `takomo` CLI uses `curl` (whose UA passes); if you write your own client, set an explicit `User-Agent` header.

## Backups (Litestream)

Continuous, off-box backup to S3-compatible storage is **prepared but off by default** — no credentials live in the repo, and the default start path is unchanged when the variables are unset.

1. Create an S3-compatible bucket (AWS S3, Cloudflare R2, MinIO, Backblaze B2, …).
2. Provide these as platform secrets / environment variables (never commit them):
   - `LITESTREAM_BUCKET` (required to activate)
   - `LITESTREAM_ACCESS_KEY_ID`, `LITESTREAM_SECRET_ACCESS_KEY`
   - optionally `LITESTREAM_ENDPOINT` (for non-AWS S3) and `LITESTREAM_REGION`
3. Run the server under Litestream:
   - **Docker image:** setting `LITESTREAM_BUCKET` is enough — [`deploy/docker-entrypoint.sh`](../deploy/docker-entrypoint.sh) wraps `serve` in `litestream replicate` and restores from the replica on a fresh disk.
   - **Elsewhere:** wrap the start command yourself:
     ```sh
     litestream replicate -config litestream.yml \
       -exec "takomo --db /var/data/takomo.db serve --bind 0.0.0.0:$PORT"
     ```

Config: [`litestream.yml`](../litestream.yml).

## Environment variables

| Variable | Purpose |
|---|---|
| `TAKOMO_ALLOW_PUBLIC_BIND` | Set to `1` to allow non-loopback binds (required when serving publicly). |
| `TAKOMO_DB` | DB path (alternative to `--db`). |
| `TAKOMO_PUBLIC_URL` | The public origin this server is reached at, e.g. `https://takomo.example.com`. **Two consumers, different strictness** (see below). Absent = OAuth off; local clients carrying a bearer token are unaffected either way. See [hosted-mcp-clients.md](hosted-mcp-clients.md). |
| `LITESTREAM_*` | Off-box backup (see above); absent = backups off. |
| `TAKOMO_GITHUB_APP_ID`, `TAKOMO_GITHUB_APP_SLUG`, `TAKOMO_GITHUB_PRIVATE_KEY_FILE` | The deployment-owned GitHub App behind Settings → GitHub and project repository extraction. All three or none; absent = the GitHub integration is off and the page says so. See [github-extraction.md](github-extraction.md). |

### `TAKOMO_PUBLIC_URL` has two readers

Worth knowing before you tighten anything around it, because the two want different things from the same string:

1. **Absolute links** in ask-a-human notifications and answer links ([ask-a-human.md](ask-a-human.md)) — the original use, and *tolerant*: any non-empty value works, trailing slashes are trimmed, a path prefix or a plain-`http` tailnet host is fine.
2. **The OAuth issuer identity** ([spec/auth.md](../spec/auth.md#oauth-21-for-hosted-mcp-clients)) — *strict*: it must be a bare `https` origin with no path, no query and no trailing slash, because a hosted client compares the issuer and the `resource` byte-for-byte against what it fetched and what the user typed. Plain `http` is accepted only on loopback.

A value that fails the strict reading **turns OAuth off and says so on the startup line; it does not stop the server.** That matters if you set this variable years ago for readable notification links: an upgrade must not take your instance down over a setting you chose for something else, having never asked for OAuth (takomo-z919). The one casualty is that hosted clients cannot connect, which the `/oauth/*` routes also report per request.

So: if you want OAuth, use a bare origin and check the startup line says `OAuth issuer …` rather than `OAuth OFF —`.

Client-side (`takomo` CLI / MCP): `TAKOMO_URL`, `TAKOMO_TOKEN`, and optionally `TAKOMO_PROJECT` / `TAKOMO_ACTOR` — usually supplied by `.takomo/config` after `takomo init`.
