#!/bin/sh
# docker-entrypoint.sh — start takomo, optionally under Litestream.
#
# Default behaviour is unchanged: exec takomo directly. Litestream is opt-in
# and engages ONLY when a replica bucket is configured (LITESTREAM_BUCKET) AND
# the container is starting the HTTP server ("serve"). Admin subcommands
# (token/project/…) always run directly.
set -eu

DB="${TAKOMO_DB:-/var/data/takomo.db}"
LITESTREAM_CONFIG="${LITESTREAM_CONFIG:-/etc/litestream.yml}"

if [ "${1:-}" = "serve" ] && [ -n "${LITESTREAM_BUCKET:-}" ]; then
  # Seed the local DB from the replica on a fresh disk (no-op if either the DB
  # already exists or no replica is present yet).
  litestream restore -if-db-not-exists -if-replica-exists -config "$LITESTREAM_CONFIG" "$DB" || true
  # Run the server as a child of litestream so writes are streamed continuously.
  exec litestream replicate -config "$LITESTREAM_CONFIG" -exec "takomo --db ${DB} $*"
fi

exec takomo --db "$DB" "$@"
