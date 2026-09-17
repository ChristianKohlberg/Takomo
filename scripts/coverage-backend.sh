#!/usr/bin/env bash
# Stable Rust supports line/region coverage. Branch coverage needs nightly;
# do not silently change the compiler just to produce another percentage.
set -euo pipefail
cd "$(dirname "$0")/.."
report_dir="${CARGO_TARGET_DIR:-target}/coverage"
mkdir -p "$report_dir"
# No hand-maintained list: every top-level integration target participates.
integration=()
for source in tests/*.rs; do
  name="${source##*/}"
  integration+=(--test "${name%.rs}")
done
for layer in unit integration combined; do
  case "$layer" in
    unit) targets=(--lib --bins) ;;
    integration) targets=("${integration[@]}") ;;
    combined) targets=(--lib --bins "${integration[@]}") ;;
  esac
  cargo llvm-cov --locked "${targets[@]}" --json --output-path "$report_dir/$layer.json"
  cargo llvm-cov report --html --output-dir "$report_dir/$layer"
done
