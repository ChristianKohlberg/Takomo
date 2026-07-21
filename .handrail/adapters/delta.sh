#!/usr/bin/env bash
# Claude Code PostToolUse hook (Write|Edit) → handrail delta.
# Only hookSpecificOutput.additionalContext reaches the model mid-loop. The touched
# path is forwarded so staleness regressions are detected without hashing.
set -uo pipefail
input="$(cat)"; sid="$(jq -r '.session_id // "default"' <<<"$input")"
tp="$(jq -r '.tool_input.file_path // .tool_input.notebook_path // empty' <<<"$input")"
out="$("${HANDRAIL_BIN:-handrail}" delta --session "$sid" ${tp:+--touched "$tp"} 2>/dev/null || true)"
[ -z "$out" ] && exit 0
jq -n --arg t "$out" '{hookSpecificOutput:{hookEventName:"PostToolUse", additionalContext:$t}}'
