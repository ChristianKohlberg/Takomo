#!/bin/sh
# backlot `auth.token` hook — mint a token against the leased database.
#
#   backlot token --role human   ->   tk_...
#
# backlot runs this in the environment tree and takes our STDOUT *verbatim* as
# the token (engine: `token: stdout.trim()`), so this must print the bare
# plaintext and nothing else. Diagnostics go to stderr.
#
# Usage: backlot-token.sh <role> <db-path>
set -eu

role="${1:?usage: backlot-token.sh <role> <db-path>}"
db="${2:?usage: backlot-token.sh <role> <db-path>}"

# Roles map onto takomo's scope model (spec/auth.md). `expert` additionally
# holds the free-form expert:<tag> scope the seeded `approve` question gates on,
# so the demo inbox is answerable end to end.
case "$role" in
  agent)  scopes="read,write" ;;
  human)  scopes="read,write,human" ;;
  expert) scopes="read,write,human,expert:domain:billing,expert:domain:product" ;;
  admin)  scopes="read,write,human,admin" ;;
  *)
    echo "backlot-token.sh: unknown role '$role' (agent | human | expert | admin)" >&2
    exit 64
    ;;
esac

bin=./target/release/takomo
if [ ! -x "$bin" ]; then
  echo "backlot-token.sh: $bin is not built yet — run 'backlot up' first" >&2
  exit 1
fi

# --json puts the plaintext in a `token` field; print just that field.
"$bin" --db "$db" token create \
  --actor "human:backlot-$role" \
  --scopes "$scopes" \
  --projects '*' \
  --json \
| python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])'
