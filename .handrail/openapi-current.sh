#!/usr/bin/env bash
# Pattern: process norm — "this kind of change is expected to leave a record."
#   HR_SURFACE   pathspec(s) whose change triggers the norm  e.g. 'src/**/Module.cs'
#   HR_RECORD    pathspec(s) where the record lives          e.g. 'docs/adr'
#   HR_MESSAGE   the expectation, restated for the output
#   HR_BASE      revision the working tree is compared against; default HEAD, i.e.
#                uncommitted work only. Point it at origin/main to check a branch
#                that is already committed — see .handrail/hr-changed.sh.
# Exit: 0 recorded, 2 nothing to check (the SKIP[...] tag says which kind),
#       1 record missing (advisory/waivable red), 3 broken invocation — nothing checked.
set -uo pipefail
SURFACE="${HR_SURFACE:?set HR_SURFACE pathspec}"
RECORD="${HR_RECORD:?set HR_RECORD pathspec}"
MESSAGE="${HR_MESSAGE:-Changes to $HR_SURFACE are expected to ship with an entry under $HR_RECORD.}"

# shellcheck source=.handrail/hr-changed.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/hr-changed.sh"
hr_resolve_base

touched="$(hr_changed "$SURFACE")"
[ -z "$touched" ] && hr_skip "$SURFACE"

recorded="$(hr_changed "$RECORD")"
if [ -n "$recorded" ]; then
  printf '%s: OK — changed under "%s" vs %s, with a record under "%s".\n' \
    "$HR_LABEL" "$SURFACE" "$HR_BASE_DESC" "$RECORD"
  exit 0
fi

echo "Surface changed with no accompanying record (vs $HR_BASE_DESC):"
# One path per line, indented. Read line-by-line rather than let an unquoted
# expansion word-split, so a path containing a space stays one entry.
while IFS= read -r p; do printf '  %s\n' "$p"; done <<<"$touched"
echo "$MESSAGE"
exit 1
