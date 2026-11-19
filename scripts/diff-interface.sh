#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
wasm="$repo_root/target/wasm32v1-none/release/tricklepay_stream.wasm"
expected="$repo_root/docs/interface.txt"

if [[ ! -f "$wasm" ]]; then
    echo "missing Wasm artifact: $wasm" >&2
    echo "run 'make wasm' before checking the interface" >&2
    exit 1
fi

if [[ ! -f "$expected" ]]; then
    echo "missing interface snapshot: $expected" >&2
    exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

stellar contract info interface --wasm "$wasm" --output rust >"$tmp"

if ! diff -u "$expected" "$tmp"; then
    echo "contract interface changed; regenerate docs/interface.txt" >&2
    exit 1
fi

echo "Interface matches."
