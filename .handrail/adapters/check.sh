#!/usr/bin/env bash
# Claude Code Stop hook → handrail check. Blocks ONCE (engine-bounded, decays on a
# clean pass) while enforcement-relevant expectations are unmet; advisory output is
# user-facing by platform design.
set -uo pipefail
input="$(cat)"; sid="$(jq -r '.session_id // "default"' <<<"$input")"
active="$(jq -r '.stop_hook_active // false' <<<"$input")"
if [ "$active" = "true" ]; then
  out="$("${HANDRAIL_BIN:-handrail}" check --session "$sid" 2>/dev/null || true)"
  [ -n "$out" ] && jq -n --arg t "$out" '{systemMessage:$t}'
  exit 0
fi
out="$("${HANDRAIL_BIN:-handrail}" check --session "$sid" --strict 2>/dev/null)"; rc=$?
[ -z "$out" ] && exit 0
if [ "$rc" -eq 2 ]; then jq -n --arg t "$out" '{decision:"block", reason:$t}'
else jq -n --arg t "$out" '{systemMessage:$t}'; fi
