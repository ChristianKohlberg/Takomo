#!/usr/bin/env bash
# Claude Code SessionStart hook → stdout is injected into the session's context.
#
# Advisory only: it reports whether the tree you just landed in is safe to build
# in, and never acquires or mutates anything. Acquiring a worktree at session
# start would chdir the session out from under the user and would mint pool
# leases for the many sessions that never need one.
#
# The rule it exists to enforce is CLAUDE.md's: several sessions work this repo
# at once, so a `cargo test` in a shared tree carrying someone else's
# uncommitted changes compiles their code together with yours and proves
# nothing. This hook makes that condition visible instead of silent.
#
# No `set -e` and no ERR trap on purpose: a hook that aborts halfway through
# leaves a truncated brief, which reads as "nothing to report". Every step
# below either guards its own failure or is harmless when it fails.
# The single-quoted printf formats below carry backticks on purpose: stdout is
# injected as markdown, so `treehouse get` should render as code, not expand.
# A file-level directive must precede the first command, hence its place here.
# shellcheck disable=SC2016
set -uo pipefail

root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
[ -n "$root" ] || exit 0

# A linked worktree has a .git FILE pointing into the parent's .git/worktrees/,
# where the main checkout has a .git directory.
if [ -f "$root/.git" ]; then
  branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null)" || branch='?'
  printf 'Worktree: isolated checkout on `%s` — safe to build here.\n' "$branch"
  exit 0
fi

dirty="$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
# Every linked worktree is a branch someone may be live on right now. The
# porcelain listing includes the main checkout, hence the -1.
linked="$(git worktree list --porcelain 2>/dev/null | grep -c '^worktree ')"
[ "$linked" -gt 0 ] && linked=$((linked - 1))

# Silence when the shared tree is clean and nothing else is in flight — a hook
# that always speaks trains you to stop reading it.
if [ "$dirty" -eq 0 ] && [ "$linked" -eq 0 ]; then
  exit 0
fi

pool_free='?'
pool_total='?'
# Read the ceiling from treehouse.toml rather than restating it here: a number
# duplicated in prose is a number that drifts.
ceiling="$(sed -n 's/^[[:space:]]*max_trees[[:space:]]*=[[:space:]]*\([0-9]\{1,\}\).*/\1/p' \
  "$root/treehouse.toml" 2>/dev/null | head -1)"
[ -n "$ceiling" ] || ceiling='default'
if command -v treehouse >/dev/null 2>&1; then
  # stdout is the table, stderr carries the update banner; ~45ms.
  if pool="$(cd "$root" && treehouse status 2>/dev/null)"; then
    pool_total="$(printf '%s\n' "$pool" | grep -c '[^[:space:]]')"
    pool_free="$(printf '%s\n' "$pool" | grep -c 'available')"
  fi
fi

printf 'Worktree: you are in the SHARED checkout (%s).\n' "$root"
if [ "$dirty" -gt 0 ]; then
  printf "  - %s uncommitted file(s) here — may be another session's work.\n" "$dirty"
fi
if [ "$linked" -gt 0 ]; then
  printf '  - %s linked worktree(s) exist; check `git worktree list` before touching a branch.\n' "$linked"
fi
printf '  - treehouse pool: %s of %s slot(s) available (ceiling %s, created on demand).\n' \
  "$pool_free" "$pool_total" "$ceiling"
printf '  Before building or verifying a branch, take a slot with `treehouse get` (subshell,\n'
printf '  self-healing). Prefer it over `--lease`, which no process liveness ever reclaims.\n'

exit 0
