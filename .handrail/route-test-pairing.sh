#!/usr/bin/env bash
# Pattern: pairing detector — "X touched without a matching Y" (SPEC kind: detector).
# Parameterize via env or edit in place after copying into your repo's .handrail/:
#   HR_PRIMARY   pathspec(s) that trigger the norm      e.g. 'src/**/*Controller.cs'
#   HR_PAIRED    pathspec(s) that must also be touched  e.g. 'tests'
#   HR_MESSAGE   the expectation, restated for the red output
#   HR_BASE      revision the working tree is compared against; default HEAD, i.e.
#                uncommitted work only. Point it at origin/main to check a branch
#                that is already committed — see .handrail/hr-changed.sh.
# Exit: 0 = paired, 2 = nothing to check (the SKIP[...] tag says which kind),
#       1 = unpaired (waivable red), 3 = broken invocation, nothing checked.
set -uo pipefail
PRIMARY="${HR_PRIMARY:?set HR_PRIMARY pathspec}"
PAIRED="${HR_PAIRED:?set HR_PAIRED pathspec}"
MESSAGE="${HR_MESSAGE:-Changes matching $HR_PRIMARY are expected to ship with a change under $HR_PAIRED.}"

# shellcheck source=.handrail/hr-changed.sh
. "$(dirname -- "${BASH_SOURCE[0]}")/hr-changed.sh"
hr_resolve_base

touched_primary="$(hr_changed "$PRIMARY")"
[ -z "$touched_primary" ] && hr_skip "$PRIMARY"

touched_paired="$(hr_changed "$PAIRED")"
if [ -n "$touched_paired" ]; then
  printf '%s: OK — changed under "%s" vs %s, with a companion change under "%s".\n' \
    "$HR_LABEL" "$PRIMARY" "$HR_BASE_DESC" "$PAIRED"
  exit 0
fi

echo "Changed without the expected companion change (vs $HR_BASE_DESC):"
# One path per line, indented. Read line-by-line rather than let an unquoted
# expansion word-split, so a path containing a space stays one entry.
while IFS= read -r p; do printf '  %s\n' "$p"; done <<<"$touched_primary"
echo "$MESSAGE"
exit 1
