# Hosting Takomo

Takomo is a single Rust + SQLite binary you run yourself. The [README](../README.md) covers the two fast paths (Deploy-to-Render button, Docker); this is the depth: local builds, TLS, and off-box backups.

## The two supported deploys

**Render (Blueprint).** [`render.yaml`](../render.yaml) provisions a `rust` web service that builds with `cargo build --release`, serves on `$PORT`, mounts a 1 GB persistent disk at `/var/data` (SQLite durability across deploys), sets `TAKOMO_ALLOW_PUBLIC_BIND=1`, and health-checks `/healthz`. Render terminates TLS for you. Deploy with the button in the README or via Dashboard → New → Blueprint.

**Docker (portable).** The [`Dockerfile`](../Dockerfile) builds a small image and also bundles [Litestream](https://litestream.io/) (dormant unless you set a bucket — see [Backups](#backups-litestream)).

```sh
docker build -t takomo .
docker run -d -p 8080:8080 -v takomo-data:/var/data --name takomo takomo
```

## Building and running from source

One binary is both the HTTP server and the `token` / `project` admin CLI.

```sh
cargo build --release
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

`/healthz` is the only unauthenticated endpoint (use it as your platform's readiness/liveness probe). Every other route requires `Authorization: Bearer tk_...`.

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
| `LITESTREAM_*` | Off-box backup (see above); absent = backups off. |

Client-side (`takomo` CLI / MCP): `TAKOMO_URL`, `TAKOMO_TOKEN`, and optionally `TAKOMO_PROJECT` / `TAKOMO_ACTOR` — usually supplied by `.takomo/config` after `takomo init`.
