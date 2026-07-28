#!/usr/bin/env bash
# Lint the JavaScript inside the two single-file SPAs.
#
# src/board.html and src/inbox.html carry ~2900 and ~2600 lines of hand-written
# JavaScript each, and until takomo-rrjg nothing in this repo looked at any of
# it: clippy has the Rust, shellcheck has the shell, Redocly has the spec, and
# `spa_string_tables_agree_on_every_key` checks exactly one property of these two
# files. A duplicate key sat in board.html's `state` literal through several
# refactors because it is legal JavaScript that no runtime complains about.
#
# The SPAs stay dependency-free — that is about what the browser downloads, and
# nothing here changes it. eslint is fetched per run by pinned `npx`, exactly the
# way the spec job already fetches @redocly/cli; no node_modules is committed and
# no script tag is added to either page.
#
# Findings are reported at the real path and the real line of the HTML file, so
# `src/board.html:874` is somewhere you can actually go. That works because the
# extracted script is padded with as many blank lines as preceded it in the HTML
# and then handed to eslint under the HTML's own name via --stdin-filename.
#
# Exit: 0 clean, 1 defect found, 2 nothing to check / cannot check.
#
# Usage: scripts/lint-spa.sh [file.html ...]   (default: both SPAs)
set -uo pipefail

ESLINT_VERSION=9.39.0

cd "$(dirname "$0")/.." || exit 2
CONFIG=scripts/spa-eslint.config.mjs

if [ "$#" -gt 0 ]; then
  FILES=("$@")
else
  FILES=(src/board.html src/inbox.html)
fi

[ -f "$CONFIG" ] || { echo "lint-spa: missing $CONFIG"; exit 2; }

command -v npx >/dev/null 2>&1 || {
  echo "skipped: npx not available, so eslint cannot be fetched (CI still lints the SPAs)"
  exit 2
}

# Probe before linting, so that a machine with no network reports "cannot check"
# rather than a red that looks like a defect. After the probe, a nonzero eslint
# is a real finding. The npx cache makes every run after the first one cheap.
if ! npx --yes "eslint@${ESLINT_VERSION}" --version >/dev/null 2>&1; then
  echo "skipped: could not obtain eslint@${ESLINT_VERSION} (offline?) — CI still lints the SPAs"
  exit 2
fi

tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT

status=0
checked=0

for file in "${FILES[@]}"; do
  if [ ! -f "$file" ]; then
    echo "lint-spa: no such file: $file"
    status=1
    continue
  fi

  # Both SPAs hold exactly one <script> block, on a line of its own. Insist on
  # that rather than coping: if a second block appears, silently linting only
  # the first would leave the new code unchecked while the job stayed green,
  # which is worse than the gate this replaces. Whoever adds one should teach
  # this script to handle it on purpose.
  opens=$(grep -c '^[[:space:]]*<script>[[:space:]]*$' "$file")
  closes=$(grep -c '^[[:space:]]*</script>[[:space:]]*$' "$file")
  if [ "$opens" -ne 1 ] || [ "$closes" -ne 1 ]; then
    echo "lint-spa: $file has $opens <script> and $closes </script> lines; expected exactly 1 of each."
    echo "  This script extracts the single inline block by line number. Teach it to walk"
    echo "  multiple blocks before adding one, or the new code ships unlinted."
    status=1
    continue
  fi

  start=$(grep -n '^[[:space:]]*<script>[[:space:]]*$' "$file" | cut -d: -f1)
  end=$(grep -n '^[[:space:]]*</script>[[:space:]]*$' "$file" | cut -d: -f1)
  if [ "$end" -le "$start" ]; then
    echo "lint-spa: $file closes its <script> at line $end, before it opens at $start."
    status=1
    continue
  fi

  # Pad with `start` blank lines so eslint's line numbers are the HTML's.
  js="$tmp/$(basename "$file").js"
  awk -v n="$start" 'BEGIN { for (i = 0; i < n; i++) print "" }' > "$js"
  sed -n "$((start + 1)),$((end - 1))p" "$file" >> "$js"

  # --max-warnings 0 is load-bearing, not tidiness. The ruleset emits only
  # errors, so nothing here should ever warn — but eslint reports "File ignored
  # because outside of base path" as a *warning* and still exits 0. Without this
  # flag, handing it a path it declines to lint prints a green "ok" for a file it
  # never opened, which is the one way a gate can be worse than not existing.
  npx --yes "eslint@${ESLINT_VERSION}" \
    --no-config-lookup --config "$CONFIG" --max-warnings 0 \
    --stdin --stdin-filename "$file" < "$js" || status=1
  checked=$((checked + 1))
done

if [ "$checked" -eq 0 ]; then
  echo "lint-spa: nothing was checked"
  [ "$status" -eq 0 ] && exit 2
fi

if [ "$status" -eq 0 ]; then
  echo "ok - eslint@${ESLINT_VERSION} clean on ${checked} SPA file(s)"
fi
exit "$status"
