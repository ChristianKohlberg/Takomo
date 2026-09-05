#!/usr/bin/env bash
# Build the frontend before Rust embeds it. Works from any current directory.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
npm --prefix web ci
npm --prefix web run build
cargo build --release "$@"
