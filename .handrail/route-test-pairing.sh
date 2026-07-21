#!/usr/bin/env bash
# Pattern: pairing detector — "X touched without a matching Y" (SPEC kind: detector).
# Parameterize via env or edit in place after copying into your repo's .handrail/:
#   HR_PRIMARY   pathspec(s) that trigger the norm      e.g. 'src/**/*Controller.cs'
#   HR_PAIRED    pathspec(s) that must also be touched  e.g. 'tests'
#   HR_MESSAGE   the expectation, restated for the red output
# Exit: 0 = paired, 2 = nothing in scope, 1 = unpaired (waivable red).
set -uo pipefail
PRIMARY="${HR_PRIMARY:?set HR_PRIMARY pathspec}"
PAIRED="${HR_PAIRED:?set HR_PAIRED pathspec}"
MESSAGE="${HR_MESSAGE:-Changes matching $HR_PRIMARY are expected to ship with a change under $HR_PAIRED.}"

touched_primary="$( { git diff --name-only HEAD -- $PRIMARY 2>/dev/null
                      git ls-files --others --exclude-standard -- $PRIMARY; } | sort -u )"
[ -z "$touched_primary" ] && exit 2

touched_paired="$( { git diff --name-only HEAD -- $PAIRED 2>/dev/null
                     git ls-files --others --exclude-standard -- $PAIRED; } | head -1 )"
[ -n "$touched_paired" ] && exit 0

echo "Changed without the expected companion change:"
printf '  %s\n' $touched_primary
echo "$MESSAGE"
exit 1
