#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec cargo run --release --manifest-path "$SCRIPT_DIR/Cargo.toml" -- "$@"
