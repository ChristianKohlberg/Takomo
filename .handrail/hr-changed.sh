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
# A file whose only difference from the base is whitespace does not count as
# changed on either side of a detector — see hr_drop_whitespace_only below for
# what that buys and, just as importantly, what it does not.
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
    if [ -n "$HR_BASE_REV" ]; then
      # core.quotePath=false: emit paths raw. The default C-style quoting of
      # non-ASCII names would produce a string that does not name the file when
      # it is fed back to git in the whitespace filter below.
      git -c core.quotePath=false diff --name-only "$HR_BASE_REV" -- "${specs[@]}" |
        hr_drop_whitespace_only
    fi
    git ls-files --others --exclude-standard -- "${specs[@]}"
  } | sort -u
}

# hr_drop_whitespace_only — filter a list of tracked paths on stdin, keeping
# only those whose difference from the base is more than whitespace.
#
# WHY this filter exists, precisely. The detectors decide "was the companion
# touched" from `git diff --name-only`, and that question is answered by the
# blob hashes: reindenting a file, or adding a blank line, makes it "touched".
# So a whitespace edit to spec/openapi.yaml used to satisfy "the spec is up to
# date", and a whitespace edit under tests/ used to satisfy "ships with a test".
# The realistic way that fires is not fraud, it is a stray reformat riding along
# with unrelated work and turning the gate green without anyone recording
# anything.
#
# What it does NOT do, and must not be sold as doing: this is still a
# touched-ness test, not a correspondence test. It cannot tell whether the spec
# change describes the route you changed, and a one-line comment or any other
# non-whitespace edit still counts as a record. The check that actually compares
# the router against the spec is a test in the suite, run by CI on every commit
# (see the route-to-spec bijection ticket); a git-diff heuristic runs only when
# someone changed something and can only ever approximate. This filter removes a
# known false PASS; it does not turn the detector into a correspondence check.
#
# Note `--name-only` does not honour `-w` — it lists every file whose blob
# differs, whatever the diff options — so each candidate is retested one at a
# time with `--quiet`, which does. `-w` alone still reports an added blank line
# as a change, hence `--ignore-blank-lines` as well.
#
# Safety property: this can only ever SHRINK the change set. A shrunk record
# side turns a false OK into red; a shrunk surface side turns a red into an
# explicit exit-2 SKIP. It can never turn a red into a pass, and it never lets
# the gate report OK on a comparison it did not make.
hr_drop_whitespace_only() {
  local path
  while IFS= read -r path; do
    # exit 1 = there is a real (non-whitespace) difference; keep the file.
    # exit 0 = whitespace-only; drop it. Anything else (a path git cannot diff)
    # is kept, so an unexpected git failure can only over-report, never hide.
    # ":(literal)" so a filename containing pathspec magic (*, ?, [, a leading
    # ':') is matched as itself and cannot silently match nothing — a pathspec
    # that matches nothing also exits 0, which would drop the file wrongly.
    if git diff -w --ignore-blank-lines --quiet "$HR_BASE_REV" -- ":(literal)$path" 2>/dev/null; then
      continue
    fi
    printf '%s\n' "$path"
  done
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
    printf '  Compared the working tree against %s and found no substantive\n' "$HR_BASE_DESC"
    printf '  difference (whitespace-only edits do not count), so this gate never saw\n'
    printf '  your change.\n'
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
