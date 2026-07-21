#!/usr/bin/env bash
# Pattern: process norm — "this kind of change is expected to leave a record."
#   HR_SURFACE   pathspec(s) whose change triggers the norm  e.g. 'src/**/Module.cs'
#   HR_RECORD    pathspec(s) where the record lives          e.g. 'docs/adr'
#   HR_MESSAGE   the expectation, restated for the output
# Exit: 0 recorded, 2 nothing in scope, 1 record missing (advisory/waivable red).
set -uo pipefail
SURFACE="${HR_SURFACE:?set HR_SURFACE pathspec}"
RECORD="${HR_RECORD:?set HR_RECORD pathspec}"
MESSAGE="${HR_MESSAGE:-Changes to $HR_SURFACE are expected to ship with an entry under $HR_RECORD.}"
touched="$( { git diff --name-only HEAD -- $SURFACE 2>/dev/null
              git ls-files --others --exclude-standard -- $SURFACE; } | sort -u )"
[ -z "$touched" ] && exit 2
recorded="$( { git diff --name-only HEAD -- $RECORD 2>/dev/null
               git ls-files --others --exclude-standard -- $RECORD; } | head -1 )"
[ -n "$recorded" ] && exit 0
echo "Surface changed with no accompanying record:"
printf '  %s\n' $touched
echo "$MESSAGE"
exit 1
