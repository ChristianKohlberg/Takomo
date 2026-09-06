#!/usr/bin/env bash
# Print whether a commit range changes packaging or release configuration.
# Missing history fails closed. Works in any Git repository (including fixtures).
set -euo pipefail
base=${1:-}
if [[ -z "$base" || "$base" =~ ^0+$ ]] || ! git cat-file -e "$base^{commit}" 2>/dev/null; then
  echo true
  exit 0
fi
changed=$(mktemp)
trap 'rm -f "$changed"' EXIT
git diff --no-renames --name-only -z "$base" HEAD > "$changed"
while IFS= read -r -d '' file; do
  case "$file" in
    Dockerfile|.dockerignore|Cargo.toml|Cargo.lock|build.rs|render.yaml|litestream.yml|deploy/*|.github/workflows/*|scripts/*|workflows/*|src/main.rs|src/server.rs|src/api/mod.rs|web/package.json|web/package-lock.json|web/vite.config.*)
      echo true
      exit 0
      ;;
  esac
done < "$changed"
echo false
