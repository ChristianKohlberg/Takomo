#!/usr/bin/env bash
# Shared change-set resolution for the pathspec detectors (route-test-pairing,
# openapi-current). SOURCED, never executed — it defines functions and exits
# only on a broken invocation.
#
# HR_BASE — optional revision the working tree is compared against.
#   unset / HEAD    the pre-commit case: uncommitted + untracked work only.
#                   This is the default, so nothing changes for a hook run.
#   origin/main     the branch case: everything this branch changed since it
#                   forked from that ref, PLUS anything still uncommitted.
#
# The comparison is always merge-base(HR_BASE, HEAD) → WORKING TREE, i.e. the
# `A...B` (three-dot) sense rather than `A B`. Two-dot against a moving `main`
# reports files that MAIN changed and this branch did not, which would make a
# detector demand a companion for somebody else's work. The merge base pins the
# fork point, so the answer does not change when main moves underneath you.
# Diffing the working TREE (not HEAD) keeps uncommitted work in view as well,
# so a branch-level run is a superset of the pre-commit run — never a different
# verdict on the same content.
#
# Exit codes seen by the caller: 3 = broken invocation (handrail reads any
# non-0/2 as red). A typo'd HR_BASE must never quietly degrade to "no changes".

# Commit to diff against; empty means "repository has no commits yet".
HR_BASE_REV=""
# Human-readable description of what we compared against, for the output.
HR_BASE_DESC=""
# Gate label, derived from the executing script so the pattern stays generic.
HR_LABEL="$(basename -- "${0%.sh}")"

hr_die_usage() { # broken invocation — loud, and never confusable with a skip
  printf '%s: CANNOT RUN — %s\n' "$HR_LABEL" "$1" >&2
  exit 3
}

hr_resolve_base() {
  local base="${HR_BASE:-HEAD}" merged
  case "$base" in
    -*) hr_die_usage "HR_BASE='$base' looks like an option, not a revision." ;;
  esac
  if ! git rev-parse -q --verify "$base^{commit}" >/dev/null 2>&1; then
    if [ "$base" = HEAD ]; then
      # No commits yet: untracked files are the whole change set. Same
      # behaviour this script had before HR_BASE existed.
      HR_BASE_REV=""
      HR_BASE_DESC="an empty repository (no commits yet)"
      return 0
    fi
    hr_die_usage "HR_BASE='$base' does not name a commit in this repository.
  Nothing was checked. Fix the ref (a branch, tag or sha — e.g. origin/main)
  and re-run; do not read this as 'no changes'."
  fi
  if ! merged="$(git merge-base "$base" HEAD 2>/dev/null)" || [ -z "$merged" ]; then
    hr_die_usage "HR_BASE='$base' and HEAD share no common ancestor, so there is
  no fork point to compare against. Nothing was checked."
  fi
  HR_BASE_REV="$merged"
  if [ "$base" = HEAD ]; then
    HR_BASE_DESC="HEAD (uncommitted work only)"
  else
    HR_BASE_DESC="$base (merge-base ${merged:0:9})"
  fi
}

# hr_changed [pathspec-string] — files differing from the base, plus untracked.
# An empty/absent argument means the whole tree. Word-splitting the argument is
# how several pathspecs are passed in one variable, so it is done deliberately
# into an array rather than left to an unquoted expansion.
hr_changed() {
  local specs=()
  read -r -a specs <<<"${1:-}"
  [ "${#specs[@]}" -eq 0 ] && specs=(".")
  {
    if [ -n "$HR_BASE_REV" ]; then git diff --name-only "$HR_BASE_REV" -- "${specs[@]}"; fi
    git ls-files --others --exclude-standard -- "${specs[@]}"
  } | sort -u
}

# hr_skip <scope-pathspecs> — called when nothing in scope changed. Separates
# the two skips that exit 2 has always conflated:
#   not-in-scope        real information: your change did not touch this surface.
#   no-changes-visible  the detector is BLIND — it saw no change at all, so it
#                       evaluated nothing. Reads as a skip, is not a pass.
# Both stay exit 2: turning a clean tree red would fire on every post-commit
# hook run, which is worse noise than the problem being fixed. The distinction
# is carried by the message and a stable SKIP[...] tag that can be grepped.
hr_skip() {
  local scope="$1" total
  total="$(hr_changed "" | grep -c . || true)"
  if [ "$total" -eq 0 ]; then
    printf '%s: SKIP[no-changes-visible] — NOT a pass, nothing was evaluated.\n' "$HR_LABEL"
    printf '  Compared the working tree against %s and found no difference at all,\n' "$HR_BASE_DESC"
    printf '  so this gate never saw your change.\n'
    if [ -n "$HR_BASE_REV" ] && [ -z "${HR_BASE:-}" ]; then
      printf '  A committed branch looks exactly like this. Name the fork point to check it:\n'
      printf '      HR_BASE=origin/main handrail run %s\n' "$HR_LABEL"
      printf '  (name the gate explicitly: --changed selects nothing on a committed tree.)\n'
    fi
  else
    printf '%s: SKIP[not-in-scope] — %s file(s) changed vs %s, none under: %s\n' \
      "$HR_LABEL" "$total" "$HR_BASE_DESC" "$scope"
    printf '  Your change did not touch this surface. Nothing to check.\n'
  fi
  exit 2
}
