#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

echo "==> Checking formatting..."
cargo fmt --check

echo "==> Running clippy lints..."
cargo clippy --locked --all-targets -- -D warnings

echo "==> Running tests..."
cargo test --locked

echo "==> All CI checks passed!"
